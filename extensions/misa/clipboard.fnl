(local definitions (require :misa.definitions))

;; Clipboard transport is independent of selection and editing. OSC 52 is the
;; portable default; command overrides use direct argv with text on stdin.

(fn clipboard-copy [config db event]
  (assert (= (type event.text) :string) "clipboard text must be a string")
  (let [clipboard {:linewise (= event.linewise true)
                   :sequence (+ (or (and db.clipboard db.clipboard.sequence) 0)
                                1)
                   :text event.text}
        effect (if config.command
                   {:argv config.command
                    :completion :clipboard/completed
                    :id (.. "clipboard:" clipboard.sequence)
                    :stdin event.text
                    :type :process/run}
                   {:text event.text :type :clipboard/write})]
    {:patch {:clipboard (misa.replace clipboard)}
     :fx [effect {:type :terminal/read}]}))

(fn clipboard-completed [db event]
  (if event.ok nil {:fx [{:event {:level :error
                                  :text (.. "Clipboard command failed: "
                                            (or event.stderr event.status
                                                "unknown error"))
                                  :type :transcript/harness}
                          :type :dispatch}]}))

(fn build [context]
  "Build the declarations for clipboard."
  (let [config (or context.config.clipboard {})]
    (definitions.build :clipboard
      [{:catalog :events
        :value {:event :clipboard/copy
                :handler (fn [db event] (clipboard-copy config db event))}}
       {:catalog :events
        :value {:event :clipboard/completed :handler clipboard-completed}}]
      {})))

{: build}
