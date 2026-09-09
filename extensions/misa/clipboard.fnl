;; Clipboard transport is independent of selection and editing. OSC 52 is the
;; portable default; command overrides use direct argv with text on stdin.

(fn clipboard-copy [config db event]
  "Describe copying text with the configured clipboard transport."
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
  "Report a failed clipboard command."
  (if event.ok nil {:fx [{:event {:level :error
                                  :text (.. "Clipboard command failed: "
                                            (or event.stderr event.status
                                                "unknown error"))
                                  :type :transcript/harness}
                          :type :dispatch}]}))

{:copy clipboard-copy :completed clipboard-completed}
