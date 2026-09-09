(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {:patch {:agent {:status :tools :pending_tools {:stale {:name :shell}}
                                                     :pending_tool_count 1}}
                                     :fx [{:event {:type :agent/reset}
                                           :type :dispatch}
                                          {:event {:text :late
                                                   :tool_call_id :stale
                                                   :type :tool/result}
                                           :type :dispatch}
                                          {:event {:type :check/reset}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :check/reset :handler (fn [db]
                                    (assert (and (and (= db.agent.status :ready)
                                                      (= (length db.agent.messages)
                                                         0))
                                                 (= db.agent.pending_tool_count
                                                    0)))
                                    {:fx [{:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.runtime-regressions-reset declarations {}))
