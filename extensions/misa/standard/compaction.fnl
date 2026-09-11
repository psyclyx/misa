(local compaction (require :misa.compaction))

(fn start [db event cofx]
  "Begin a summarization request through the framework."
  (compaction.start cofx.config db event))

(fn check [db event cofx]
  "Start automatic compaction when the conversation exceeds its budget."
  (compaction.check cofx.config db event))

(fn stream-delta [db event cofx]
  "Accumulate streamed summary text."
  (compaction.stream-delta cofx.config db event))

(fn stream-end [db event cofx]
  "Apply a completed summary."
  (compaction.stream-end cofx.config db event))

(fn stream-error [db event cofx]
  "Finish a failed summary request."
  (compaction.stream-error cofx.config db event))

{:actions {:compaction.compact {:available (fn [db] (not db.dialog))
                                :event {:type :compaction/request}
                                :id :compaction.compact
                                :label "Compact conversation"}}
 :commands {:/compact {:description "Summarize and replace the conversation history"
                       :event :compaction/request
                       :name :/compact}}
 :events {:compaction/agent/status {:event :agent/status
                                    :handler check
                                    :priority 56000}
          :compaction/request {:event :compaction/request
                               :handler compaction.on-request
                               :priority 55000}
          :compaction/start {:event :compaction/start
                             :handler start
                             :priority 55000}
          :compaction/cancel {:event :compaction/cancel
                              :handler compaction.cancel
                              :priority 55000}
          :compaction/agent/cancel {:event :agent/cancel-active
                                    :handler compaction.cancel
                                    :priority 55000}
          :compaction/stream-delta {:event :agent/stream-delta
                                    :handler stream-delta
                                    :priority 55000}
          :compaction/stream-usage {:event :agent/stream-usage
                                    :handler compaction.stream-usage
                                    :priority 55000}
          :compaction/stream-end {:event :agent/stream-end
                                  :handler stream-end
                                  :priority 55000}
          :compaction/stream-result {:event :agent/result
                                     :handler stream-end
                                     :priority 55000}
          :compaction/stream-error {:event :agent/stream-error
                                    :handler stream-error
                                    :priority 55000}
          :compaction/model/open {:event :model/open
                                  :handler compaction.model-changed
                                  :priority 55000}
          :compaction/model/select {:event :model/select
                                    :handler compaction.model-changed
                                    :priority 55000}
          :compaction/model/selection-loaded {:event :model/selection-loaded
                                              :handler compaction.model-changed
                                              :priority 55000}
          :compaction/models/update {:event :models/update
                                     :handler compaction.model-changed
                                     :priority 55000}
          :compaction/models/provider-availability {:event :models/provider-availability
                                                    :handler compaction.model-changed
                                                    :priority 55000}
          :compaction/models/replace-provider {:event :models/replace-provider
                                               :handler compaction.model-changed
                                               :priority 55000}
          :compaction/reconcile {:event :compaction/reconcile
                                 :handler compaction.reconcile
                                 :priority 55000}
          :compaction/agent/reset {:event :agent/reset
                                   :handler compaction.reset
                                   :priority 55000}
          :compaction/transcript/reset {:event :transcript/reset
                                        :handler compaction.reset
                                        :priority 55000}}
 :services {:editor.lifecycle.compaction [:compaction/lifecycle]}
 :subscriptions {:compaction/lifecycle {:id :compaction/lifecycle
                                        :inputs [[:db/path :compaction :active]]
                                        :compute compaction.lifecycle}
                 :compaction/active {:id :compaction/active
                                     :inputs [[:db/path :compaction :active]]
                                     :compute (fn [inputs] (. inputs 1))}}}
