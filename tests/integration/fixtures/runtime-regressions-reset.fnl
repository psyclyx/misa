{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (set db.agent.status :tools)
                                    (set db.agent.pending_tools
                                         {:stale {:name :shell}})
                                    (set db.agent.pending_tool_count 1)
                                    {: db
                                     :fx [{:event {:type :agent/reset}
                                           :type :dispatch}
                                          {:event {:text :late
                                                   :tool_call_id :stale
                                                   :type :tool/result}
                                           :type :dispatch}
                                          {:event {:type :check/reset}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :check/reset
                         :handler (fn [db]
                                    (assert (and (and (= db.agent.status :ready)
                                                      (= (length db.agent.messages)
                                                         0))
                                                 (= db.agent.pending_tool_count
                                                    0)))
                                    {:fx [{:type :app/quit}]})})
          nil
          {:fx setup-fx})}
