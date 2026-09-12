(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn []
                                    {:fx [{:event {:response_id :metadata
                                                   :model :metadata/model
                                                   :role :assistant
                                                   :type :transcript/response-start}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/1
                                                   :kind :assistant
                                                   :response_id :metadata
                                                   :type :transcript/block-start}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/1
                                                   :response_id :metadata
                                                   :text :first
                                                   :type :transcript/block-delta}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/1
                                                   :response_id :metadata
                                                   :type :transcript/block-end}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/2
                                                   :call_id :call
                                                   :kind :tool_call
                                                   :name :demo
                                                   :response_id :metadata
                                                   :type :transcript/block-start}
                                           :type :dispatch}
                                          {:event {:arguments_json_delta "{\"value\":"
                                                   :block_id :metadata/2
                                                   :response_id :metadata
                                                   :type :transcript/block-delta}
                                           :type :dispatch}
                                          {:event {:arguments_json_delta "1}"
                                                   :block_id :metadata/2
                                                   :response_id :metadata
                                                   :type :transcript/block-delta}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/2
                                                   :response_id :metadata
                                                   :type :transcript/block-end}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/3
                                                   :kind :assistant
                                                   :response_id :metadata
                                                   :type :transcript/block-start}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/3
                                                   :response_id :metadata
                                                   :text :second
                                                   :type :transcript/block-delta}
                                           :type :dispatch}
                                          {:event {:block_id :metadata/3
                                                   :response_id :metadata
                                                   :type :transcript/block-end}
                                           :type :dispatch}
                                          {:completion :test/metadata-finish
                                           :id :metadata-delay
                                           :interval_ms 20
                                           :type :timer/start}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :test/metadata-finish
                         :handler (fn []
                                    {:fx [{:id :metadata-delay
                                           :type :timer/stop}
                                          {:event {:response_id :metadata
                                                   :type :transcript/response-end
                                                   :usage {:output_tokens 12
                                                           :cost_usd 0.125}}
                                           :type :dispatch}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :transcript/response-end
                         :handler (fn [db event]
                                    (if (not= event.response_id :metadata) nil
                                        (do
                                          (local tool (. db.messages.blocks 2))
                                          (assert (and (= tool.argument_chunks
                                                          nil)
                                                       (= tool.argument_text
                                                          "{\"value\":1}"))
                                                  "final tool argument chunks were not compacted")
                                          (local cost
                                                 (misa.costs.group db :metadata))
                                          (assert (= cost.type :money))
                                          (assert (= cost.amount 0.125))
                                          (assert (= cost.text nil))
                                          (local layout
                                                 (misa.transcript.layout db
                                                                         {:columns 80
                                                                          :interactive true}))
                                          (var metadata-count 0)
                                          (each [_ item (ipairs layout.items)]
                                            (assert (= item.model.timestamp nil)
                                                    "timestamp was formatted upstream")
                                            (assert (= (type item.model.started_wall_ms)
                                                       :number))
                                            (when (= item.role
                                                     :transcript.group_footer)
                                              (assert (= item.model.cost cost)
                                                      "cost fact lost identity")
                                              (assert (= item.id
                                                         "response:metadata:footer"))
                                              (assert (> item.model.tokens_per_second
                                                         0))
                                              (set metadata-count
                                                   (+ metadata-count 1))))
                                          (assert (= metadata-count 1))
                                          (local lines
                                                 (misa.transcript.project db
                                                                          {:columns 80
                                                                           :interactive true}))
                                          (assert (= metadata-count 1))
                                          (var (rates costs) (values 0 0))
                                          (each [_ line (ipairs lines)]
                                            (each [_ item (ipairs line.spans)]
                                              (when (item.text:find :tok/s 1
                                                                    true)
                                                (set rates (+ rates 1)))
                                              (when (item.text:find :$0.1250 1
                                                                    true)
                                                (set costs (+ costs 1)))))
                                          (assert (= rates 1)
                                                  "response tok/s metadata was missing or rendered more than once")
                                          (assert (= costs 1)
                                                  "response cost was missing or rendered more than once")
                                          {:fx [{:lines [{:spans [{:text :metadata}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))}})
  nil
  (definitions.collect :tests.integration.fixtures.response-metadata
    declarations
    {}))
