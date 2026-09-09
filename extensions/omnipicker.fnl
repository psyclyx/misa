(local definitions (require :misa.definitions))

;; Command palette composed from registered commands and canonical recent
;; invocations. Argument completion is an ordinary narrowing source.

(fn command-items [db]
  (let [(result seen) (values {} {})]
    (each [_ command (ipairs (misa.commands.all))]
      (var narrow nil)
      (when (or command.completion command.complete)
        (set narrow {:context {:command command.name}
                     :source :command-arguments}))
      (tset result (+ (length result) 1)
            {:display {:description command.description :label command.name}
             :id command.name
             :invocation (or (and (not narrow) command.name) nil)
             : narrow
             :path command.name
             :preview {:description command.description :title command.name}
             :search [command.name command.description]
             :value command.name})
      (tset seen command.name true))
    (each [_ entry (ipairs (misa.commands.recent db))]
      (when (not (. seen entry.canonical))
        (tset seen entry.canonical true)
        (tset result (+ (length result) 1)
              {:display {:description "recent invocation"
                         :label entry.canonical}
               :id entry.canonical
               :invocation entry.canonical
               :path entry.canonical
               :preview {:kind :recent :title entry.canonical}
               :search [entry.command entry.arguments]
               :value entry.canonical})))
    result))

(fn []
  "Build the declarations for omnipicker."
  (local declarations [])
  (table.insert declarations
                {:catalog :choice-sources
                 :id :omnipicker
                 :value {:items (fn [_ db]
                                  {:input_prefix "/"
                                   :items (command-items db)
                                   :preference_scope :commands
                                   :purpose :command-completion
                                   :title :Commands})}})
  (table.insert declarations
                {:catalog :choice-sources
                 :id :command-arguments
                 :value {:items (fn [context db]
                                  (misa.commands.choice-spec (assert (misa.commands.lookup context.command))
                                                             "" db))}})
  (table.insert declarations
                {:catalog :services
                 :id :picker.session
                 :value (fn [db query]
                          "Create a choice session for the unified picker."
                          (local spec (misa.choices.source :omnipicker nil db))
                          (set spec.query (or query ""))
                          (misa.choices.session spec db))})
  (table.insert declarations
                (let [definition {:action :open_omnipicker
                                  :context :global
                                  :default [:alt+/]}]
                  {:catalog :keybindings
                   :id (.. (. definition :context) "/" (. definition :action))
                   :value definition}))
  (table.insert declarations
                (let [definition {:binding {:action :open_omnipicker
                                            :context :global}
                                  :event {:type :omnipicker/open}
                                  :id :commands.open
                                  :label "Open session commands"}]
                  {:catalog :actions :id (. definition :id) :value definition}))
  (table.insert declarations
                {:catalog :events
                 :value {:event :omnipicker/open
                         :handler (fn [db]
                                    (local sequence
                                           (+ (or db.omnipicker_sequence 0) 1))
                                    (local token (.. "omnipicker:" sequence))
                                    {:patch {:omnipicker_sequence sequence}
                                     :fx [{:event {:completion :omnipicker/selected
                                                   :id :omnipicker
                                                   :nested (not= db.picker nil)
                                                   :session (misa.picker.session db
                                                                                 "")
                                                   :title :Commands
                                                   : token
                                                   :type :picker/open}
                                           :type :dispatch}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :omnipicker/selected
                         :handler (fn [db event]
                                    (if (not= event.picker :omnipicker) nil
                                        (if event.cancelled
                                            {:fx [{:type :terminal/read}]}
                                            (do
                                              (local invocation
                                                     (assert (misa.commands.invocation (or event.invocation
                                                                                           (tostring event.value)))
                                                             "invalid command palette invocation"))
                                              {:fx [{:event invocation
                                                     :type :dispatch}]}))))}})
  (definitions :omnipicker
    declarations
    {:requirements {:omnipicker [:commands.invocation]}}))
