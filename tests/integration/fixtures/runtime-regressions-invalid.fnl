{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/completed
                         :handler (fn [db]
                                    (assert (and (= db.agent.request_seq 2)
                                                 (= db.agent.status :ready)))
                                    (assert (. db.agent.messages 3 :is_error)
                                            "invalid tool input did not become a tool error")
                                    {:fx [{:type :app/quit}]})})
          nil
          {:fx setup-fx})}
