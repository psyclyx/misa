(local definitions (require :misa.definitions))

(fn [context]
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {:patch {:phase :INTERMEDIATE_START :count 0 :typed ""}
                                     :fx [{:event {:type :fixture/settle}
                                           :type :dispatch}
                                          {:type :terminal/read}
                                          {:argv [context.config.python
                                                  :-u
                                                  context.config.producer]
                                           :completion :fixture/stream
                                           :id :producer
                                           :stdout_format :json_lines_stream
                                           :type :process/run}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :fixture/stream :handler (fn [db event]
                                    (if event.records
                                        (do
                                          {:patch {:phase :INTERMEDIATE_STREAM
                                                   :count (+ db.count (length event.records))}
                                           :fx [{:event {:type :fixture/settle}
                                                 :type :dispatch}]})
                                        (= event.phase :end)
                                        {:patch {:done true}}
                                        nil))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :fixture/settle :handler (fn [_] {:patch {:phase :SETTLED}})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :terminal/input :handler (fn [db event]
                                    (if (= event.kind :ctrl_d)
                                        {:fx [{:type :app/quit}]}
                                        (if (= event.kind :text)
                                            (do
                                              {:patch {:typed (.. db.typed event.text) :phase :INTERMEDIATE_INPUT}
                                               :fx [{:event {:type :fixture/settle}
                                                     :type :dispatch}
                                                    {:type :terminal/read}]})
                                            {:fx [{:type :terminal/read}]})))}})
          (table.insert declarations
                        {:catalog :views :id :main :value (fn [db]
                                    {:lines [{:spans [{:text (.. (or db.phase
                                                                     "")
                                                                 " "
                                                                 (or db.typed
                                                                     "")
                                                                 " count="
                                                                 (tostring (or db.count
                                                                               0))
                                                                 (or (and db.done
                                                                          " DONE")
                                                                     ""))}]}]})})
          nil
          (definitions :tests.settled-frames declarations {}))
