{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/completed
                         :handler (fn [db]
                                    (assert (= db.agent.request_seq 1)
                                            "provider tool loop was executed twice")
                                    (assert (and (= db.agent.status :ready)
                                                 (= db.agent.pending_tool_count
                                                    0)))
                                    (assert (= (length db.agent.messages) 4)
                                            "provider tool results missing from history")
                                    (local blocks
                                           (. db.agent.messages 2 :content))
                                    (assert (and (= (. blocks 1 :id) :one)
                                                 (= (. blocks 2 :id) :two))
                                            "provider message indices merged distinct tool calls")
                                    (assert (and (= (. blocks 1 :arguments
                                                       :command)
                                                    :first)
                                                 (= (. blocks 2 :arguments
                                                       :command)
                                                    :second))
                                            (misa.json.encode blocks))
                                    (assert (= (. db.agent.messages 3 :content
                                                  1 :text)
                                               "first result"))
                                    (assert (= (. db.agent.messages 4 :content
                                                  1 :text)
                                               "second result"))
                                    {:fx [{:type :app/quit}]})})
          nil
          {:fx setup-fx})}
