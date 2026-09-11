(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn []
                                    {:fx [{:event {:response_id :rate
                                                   :role :assistant
                                                   :type :transcript/response-start}
                                           :type :dispatch}
                                          {:event {:block_id :rate/1
                                                   :kind :assistant
                                                   :response_id :rate
                                                   :type :transcript/block-start}
                                           :type :dispatch}
                                          {:event {:block_id :rate/1
                                                   :response_id :rate
                                                   :text :rated
                                                   :type :transcript/block-delta}
                                           :type :dispatch}
                                          {:completion :rate/finish
                                           :id :rate
                                           :interval_ms 10
                                           :type :timer/start}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :rate/finish
                         :handler (fn []
                                    {:fx [{:id :rate :type :timer/stop}
                                          {:event {:block_id :rate/1
                                                   :response_id :rate
                                                   :type :transcript/block-end}
                                           :type :dispatch}
                                          {:event {:response_id :rate
                                                   :type :transcript/response-end
                                                   :usage {:output_tokens 20}}
                                           :type :dispatch}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :transcript/response-end
                         :handler (fn [db event]
                                    (if (not= event.response_id :rate) nil
                                        (do
                                          (local response
                                                 (. db.messages.responses 1))
                                          (assert (and (and (= response.output_tokens
                                                               20)
                                                            (> response.elapsed_ms
                                                               0))
                                                       (= response.tokens_per_second
                                                          (/ 20000
                                                             response.elapsed_ms)))
                                                  "completion rate did not use reported output tokens and monotonic elapsed time")
                                          {:fx [{:type :app/quit}]})))}})
  nil
  (definitions.collect :tests.integration.fixtures.rate-check declarations {}))
