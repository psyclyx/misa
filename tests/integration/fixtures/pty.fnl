(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (local request-id :pty-operation)
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [_ _ cofx]
                                    (var producer
                                         "printf '{\"part\":1}\\n'; sleep 0.70; printf '{\"part\":2}\\n'")
                                    (when cofx.config.cancel
                                      (set producer
                                           (.. "echo $$ >'" cofx.config.pid
                                               "'; printf '{\"part\":1}\\n'; sleep 10")))
                                    {:fx [{:argv [:/bin/sh :-c producer]
                                           :completion :pty/stream
                                           :id request-id
                                           :stdout_format :json_lines_stream
                                           :type :process/run}
                                          {:type :terminal/read}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :pty/stream :handler (fn [_ event]
                                    (if (and (= event.phase :data)
                                             (> (length (or event.records {}))
                                                0))
                                        {:fx [{:lines [{:spans [{:text "partial transcript"}]}]
                                               :type :view/commit}]}
                                        (if (= event.phase :end)
                                            (do
                                              (local text
                                                     (or (and (= event.message
                                                                 :Canceled)
                                                              "cancelled child")
                                                         "producer completed"))
                                              {:fx [{:lines [{:spans [{: text}]}]
                                                     :type :view/commit}
                                                    {:type :app/quit}]})
                                            nil)))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :terminal/resize :handler (fn [_ event]
                                    {:fx [{:lines [{:spans [{:text (.. "resized "
                                                                       event.columns
                                                                       :x
                                                                       event.lines)}]}]
                                           :type :view/commit}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :terminal/input :handler (fn [_ event]
                                    (if (= event.kind :ctrl_c)
                                        {:fx [{:id request-id
                                               :type :operation/cancel}]}
                                        {:fx [{:type :terminal/read}]}))}})
          nil
          (definitions :tests.integration.fixtures.pty declarations {}))
