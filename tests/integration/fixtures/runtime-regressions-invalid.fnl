(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/completed
                         :handler (fn [db]
                                    (assert (and (= db.agent.request_seq 2)
                                                 (= db.agent.status :ready)))
                                    (assert (. db.agent.messages 3 :is_error)
                                            "invalid tool input did not become a tool error")
                                    {:fx [{:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.runtime-regressions-invalid
    declarations
    {}))
