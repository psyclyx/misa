(local definitions (require :misa.definitions))

;; Command provider for tests and user-supplied model adapters.

(fn latest-prompt [messages]
  (let [message (. messages (length messages))]
    (assert (and message (= message.role :user))
            "command provider requires a final user message")
    (local parts {})
    (each [_ block (ipairs message.content)]
      (when (= block.type :text)
        (tset parts (+ (length parts) 1) block.text)))
    (local prompt (table.concat parts "\n"))
    (assert (not= prompt "") "command prompt must be nonempty")
    prompt))

(fn [context]
  "Describe command policies for the supplied application settings."
  (local declarations [])
  (local providers (or (and (= (type context.config) :table)
                            context.config.providers)
                       nil))
  (local command (or (and (= (type providers) :table) providers.command) nil))
  (local configured (or (and (= (type command) :table) command.argv) nil))
  (assert (and (= (type configured) :table) (> (length configured) 0))
          "config.providers.command.argv must be a nonempty array")
  (local argv {})
  (for [i 1 (length configured)]
    (assert (and (= (type (. configured i)) :string) (not= (. configured i) ""))
            "command argv must contain nonempty strings")
    (assert (not (: (. configured i) :find "\000" 1 true))
            "command argv must not contain NUL")
    (tset argv i (. configured i)))
  (table.insert declarations
                (let [definition {:id :command/default
                                  :label :Command
                                  :model :default
                                  :provider :command}]
                  {:catalog :models :id (. definition :id) :value definition}))
  (table.insert declarations
                {:catalog :effects
                 :id :provider.command
                 :value (fn [effect]
                          (assert (and (= (type effect.id) :string)
                                       (not= effect.id ""))
                                  "command id must be a nonempty string")
                          (local direct {})
                          (for [i 1 (length argv)]
                            (tset direct i (. argv i)))
                          (tset direct (+ (length direct) 1)
                                (latest-prompt effect.messages))
                          {:argv direct
                           :completion :provider/command-complete
                           :id effect.id
                           :type :provider/process})})
  (table.insert declarations
                {:catalog :events
                 :value {:event :provider/command-complete
                         :handler (fn [_ event]
                                    (assert (and (= (type event.id) :string)
                                                 (not= event.id ""))
                                            "command completion id must be nonempty")
                                    (local fx
                                           [{:event {:id event.id
                                                     :type :agent/stream-start}
                                             :type :dispatch}])
                                    (if event.ok
                                        (do
                                          (tset fx (+ (length fx) 1)
                                                {:event {:delta {:text event.stdout
                                                                 :type :text}
                                                         :id event.id
                                                         :type :agent/stream-delta}
                                                 :type :dispatch})
                                          (tset fx (+ (length fx) 1)
                                                {:event {:id event.id
                                                         :type :agent/stream-end}
                                                 :type :dispatch}))
                                        (tset fx (+ (length fx) 1)
                                              {:event {:id event.id
                                                       :message (or (and (not= event.stderr
                                                                               "")
                                                                         event.stderr)
                                                                    (.. "command exited "
                                                                        (tostring event.status)))
                                                       :type :agent/stream-error}
                                               :type :dispatch}))
                                    {: fx})}})
  (definitions :provider.command declarations {}))
