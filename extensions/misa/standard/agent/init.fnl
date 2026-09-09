(local agent (require :misa.agent))

{:agent-deltas {:text agent.text-delta
                :thinking agent.text-delta
                :tool_call agent.tool-delta}
 :validators {:agent-deltas (fn [_ value]
                              (assert (= (type value) :function)
                                      "agent delta projection must be a function"))}
 :commands {:/clear {:description "Reset conversation and token usage"
                     :event :agent/reset
                     :name :/clear}}
 :events {:agent/start {:event :app/start
                        :priority 63000
                        :handler (fn [db event cofx]
                                   (agent.start (or cofx.config.agent {}) db
                                                event cofx))}
          :agent/startup {:event :agent/startup
                          :priority 63000
                          :handler agent.continue-startup}
          :agent/auth-ready {:event :auth/startup-ready
                             :priority 63000
                             :handler agent.continue-startup}
          :agent/cancel {:event :agent/cancel-active
                         :priority 63000
                         :handler agent.cancel}
          :agent/reset {:event :agent/reset
                        :priority 63000
                        :handler agent.reset}
          :agent/submit {:event :agent/submit
                         :priority 63000
                         :handler agent.submit}
          :agent/stream-start {:event :agent/stream-start
                               :priority 63000
                               :handler agent.stream-start}
          :agent/stream-delta {:event :agent/stream-delta
                               :priority 63000
                               :handler agent.stream-delta}
          :agent/stream-tool-result {:event :agent/stream-tool-result
                                     :priority 63000
                                     :handler agent.stream-tool-result}
          :agent/stream-usage {:event :agent/stream-usage
                               :priority 63000
                               :handler agent.stream-usage}
          :agent/stream-state {:event :agent/stream-state
                               :priority 63000
                               :handler agent.stream-state}
          :agent/stream-end {:event :agent/stream-end
                             :priority 63000
                             :handler agent.stream-end}
          :agent/legacy-result {:event :agent/result
                                :priority 63000
                                :handler agent.legacy-result}
          :agent/tool-result {:event :tool/result
                              :priority 63000
                              :handler agent.receive-tool-result}
          :agent/stream-error {:event :agent/stream-error
                               :priority 63000
                               :handler agent.stream-error}
          :agent/error {:event :agent/error
                        :priority 63000
                        :handler agent.stream-error}}}
