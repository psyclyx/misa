{:setup (fn [context]
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (set db.phase :INTERMEDIATE_START)
                                    (set db.count 0)
                                    (set db.typed "")
                                    {: db
                                     :fx [{:event {:type :fixture/settle}
                                           :type :dispatch}
                                          {:type :terminal/read}
                                          {:argv [context.config.python
                                                  :-u
                                                  context.config.producer]
                                           :completion :fixture/stream
                                           :id :producer
                                           :stdout_format :json_lines_stream
                                           :type :process/run}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :fixture/stream
                         :handler (fn [db event]
                                    (if event.records
                                        (do
                                          (set db.phase :INTERMEDIATE_STREAM)
                                          (set db.count
                                               (+ db.count
                                                  (length (or event.records {}))))
                                          {: db
                                           :fx [{:event {:type :fixture/settle}
                                                 :type :dispatch}]})
                                        (= event.phase :end)
                                        (do
                                          (set db.done true)
                                          {: db})
                                        nil))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :fixture/settle
                         :handler (fn [db] (set db.phase :SETTLED) {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :terminal/input
                         :handler (fn [db event]
                                    (if (= event.kind :ctrl_d)
                                        {:fx [{:type :app/quit}]}
                                        (if (= event.kind :text)
                                            (do
                                              (set db.typed
                                                   (.. db.typed event.text))
                                              (set db.phase :INTERMEDIATE_INPUT)
                                              {: db
                                               :fx [{:event {:type :fixture/settle}
                                                     :type :dispatch}
                                                    {:type :terminal/read}]})
                                            {:fx [{:type :terminal/read}]})))})
          (table.insert setup-fx
                        {:type :register/view
                         :handler (fn [db]
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
          {:fx setup-fx})}
