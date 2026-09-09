(local tools (require :misa.transcript.tools))
(local summary (require :misa.transcript.tools.summary))

{:services {:tools.presentation tools.tools-presentation}
 :tool-presentations {:shell {:fields [:command]
                              :code :command
                              :language :sh
                              :numbered false
                              :result tools.shell-result}
                      :read_file {:subject :path
                                  :fields [:path]
                                  :result tools.read-result}
                      :list_directory {:subject :path :fields [:path]}
                      :write_file {:subject :path
                                   :fields [:path :content]
                                   :code :content}
                      :edit_file tools.edit-view}
 :validators {:tool-presentations tools.validate-binding}
 :events {:tool_summary/transcript/tool-result {:event :transcript/tool-result
                                                :handler summary.transcript-tool-result}
          :tool_summary/tool-summary/next {:event :tool-summary/next
                                           :handler summary.tool-summary-next}
          :tool_summary/agent/stream-delta {:event :agent/stream-delta
                                            :handler summary.agent-stream-delta}
          :tool_summary/agent/stream-usage {:event :agent/stream-usage
                                            :handler summary.agent-stream-usage}
          :tool_summary/model/role {:event :model/role
                                    :handler summary.model-changed}
          :tool_summary/model/roles-loaded {:event :model/roles-loaded
                                            :handler summary.model-changed}
          :tool_summary/models/update {:event :models/update
                                       :handler summary.model-changed}
          :tool_summary/models/provider-availability {:event :models/provider-availability
                                                      :handler summary.model-changed}
          :tool_summary/models/replace-provider {:event :models/replace-provider
                                                 :handler summary.model-changed}
          :tool_summary/tool-summary/reconcile {:event :tool-summary/reconcile
                                                :handler summary.tool-summary-reconcile}
          :tool_summary/agent/result {:event :agent/result
                                      :handler (fn [db event]
                                                 (summary.finish db event false))}
          :tool_summary/agent/stream-end {:event :agent/stream-end
                                          :handler (fn [db event]
                                                     (summary.finish db event
                                                                     false))}
          :tool_summary/agent/stream-error {:event :agent/stream-error
                                            :handler (fn [db event]
                                                       (summary.finish db event
                                                                       true))}
          :tool_summary/agent/reset {:event :agent/reset
                                     :handler summary.reset}
          :tool_summary/transcript/reset {:event :transcript/reset
                                          :handler summary.reset}}}
