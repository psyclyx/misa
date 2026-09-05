{:setup (fn [context]
          (misa.reg_event :app/start
                          (fn [db]
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
                                   :type :process/run}]}))
          (misa.reg_event :fixture/stream
                          (fn [db event]
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
                                  {: db}) nil)))
          (misa.reg_event :fixture/settle
                          (fn [db] (set db.phase :SETTLED) {: db}))
          (misa.reg_event :terminal/input
                          (fn [db event]
                            (if (= event.kind :ctrl_d)
                                {:fx [{:type :app/quit}]}
                                (if (= event.kind :text)
                                    (do
                                      (set db.typed (.. db.typed event.text))
                                      (set db.phase :INTERMEDIATE_INPUT)
                                      {: db
                                       :fx [{:event {:type :fixture/settle}
                                             :type :dispatch}
                                            {:type :terminal/read}]})
                                    {:fx [{:type :terminal/read}]}))))
          (misa.reg_view (fn [db]
                           {:lines [{:spans [{:text (.. (or db.phase "") " "
                                                        (or db.typed "")
                                                        " count="
                                                        (tostring (or db.count
                                                                      0))
                                                        (or (and db.done
                                                                 " DONE")
                                                            ""))}]}]}))
          nil)}

