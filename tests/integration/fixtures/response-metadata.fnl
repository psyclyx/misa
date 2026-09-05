{:setup (fn []
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:event {:response_id :metadata
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
                                   :type :timer/start}]}))
          (misa.reg_event :test/metadata-finish
                          (fn []
                            {:fx [{:id :metadata-delay :type :timer/stop}
                                  {:event {:response_id :metadata
                                           :type :transcript/response-end
                                           :usage {:output_tokens 12}}
                                   :type :dispatch}]}))
          (misa.reg_event :transcript/response-end
                          (fn [db event]
                            (if (not= event.response_id :metadata) nil
                                (do
                                  (local tool (. db.messages.blocks 2))
                                  (assert (and (= tool.argument_chunks nil)
                                               (= tool.argument_text
                                                  "{\"value\":1}"))
                                          "final tool argument chunks were not compacted")
                                  (local lines
                                         (misa.transcript_projection db
                                                                     {:columns 80
                                                                      :interactive true}))
                                  (var rates 0)
                                  (each [_ line (ipairs lines)]
                                    (each [_ item (ipairs line.spans)]
                                      (when (item.text:find :tok/s 1 true)
                                        (set rates (+ rates 1)))))
                                  (assert (= rates 1)
                                          "response tok/s metadata was missing or rendered more than once")
                                  {:fx [{:lines [{:spans [{:text :metadata}]}]
                                         :type :view/commit}
                                        {:type :app/quit}]}))))
          nil)}

