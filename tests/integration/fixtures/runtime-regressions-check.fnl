(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :transcript/block-start
                         :handler (fn [db event]
                                    (when (= event.kind :tool_call)
                                      {:patch {:test_tool_starts (+ (or db.test_tool_starts
                                                                        0)
                                                                    1)}}))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/completed
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
                                    (assert (= (length blocks) 3)
                                            "final records duplicated agent content")
                                    (assert (= db.test_tool_starts 2)
                                            "tool calls were displayed twice")
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
                                    {:fx [{:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.runtime-regressions-check
    declarations
    {}))
