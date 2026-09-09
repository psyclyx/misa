(local definitions (require :misa.definitions))

;; Clipboard transport is independent of selection and editing. OSC 52 is the
;; portable default; command overrides use direct argv with text on stdin.

(fn [context]
  "Build the declarations for clipboard."
  (local declarations [])
  (local config (or context.config.clipboard {}))
  (table.insert declarations
                {:catalog :events
                 :value {:event :clipboard/copy
                         :handler (fn [db event]
                                    (assert (= (type event.text) :string)
                                            "clipboard text must be a string")
                                    (local clipboard
                                           {:linewise (= event.linewise true)
                                            :sequence (+ (or (. (or db.clipboard
                                                                    {})
                                                                :sequence)
                                                             0)
                                                         1)
                                            :text event.text})
                                    (var effect
                                         {:text event.text
                                          :type :clipboard/write})
                                    (when config.command
                                      (set effect
                                           {:argv config.command
                                            :completion :clipboard/completed
                                            :id (.. "clipboard:"
                                                    clipboard.sequence)
                                            :stdin event.text
                                            :type :process/run}))
                                    {:patch {:clipboard (misa.replace clipboard)}
                                     :fx [effect {:type :terminal/read}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :clipboard/completed
                         :handler (fn [db event]
                                    (if event.ok nil
                                        {:fx [{:event {:level :error
                                                       :text (.. "Clipboard command failed: "
                                                                 (or event.stderr
                                                                     event.status
                                                                     "unknown error"))
                                                       :type :transcript/harness}
                                               :type :dispatch}]}))}})
  (definitions :clipboard declarations {}))
