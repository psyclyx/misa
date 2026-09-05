;; Clipboard transport is independent of selection and editing. OSC 52 is the

;; portable default; command overrides use direct argv with text on stdin.

{:setup (fn [context]
          (local setup-fx [])
          (local config (or context.config.clipboard {}))
          (table.insert setup-fx
                        {:type :register/event
                         :name :clipboard/copy
                         :handler (fn [db event]
                                    (assert (= (type event.text) :string)
                                            "clipboard text must be a string")
                                    (set db.clipboard
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
                                                    db.clipboard.sequence)
                                            :stdin event.text
                                            :type :process/run}))
                                    {: db :fx [effect {:type :terminal/read}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :clipboard/completed
                         :handler (fn [db event]
                                    (if event.ok {: db}
                                        {: db
                                         :fx [{:event {:level :error
                                                       :text (.. "Clipboard command failed: "
                                                                 (or (or event.stderr
                                                                         event.status)
                                                                     "unknown error"))
                                                       :type :transcript/harness}
                                               :type :dispatch}]}))})
          {:fx setup-fx})}
