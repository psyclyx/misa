(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (local lifecycle {:block_end 0
                            :block_start 0
                            :delta 0
                            :finish 0
                            :start 0})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/response-start :handler (fn []
                                    (set lifecycle.start (+ lifecycle.start 1))
                                    nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/block-start :handler (fn []
                                    (set lifecycle.block_start
                                         (+ lifecycle.block_start 1))
                                    nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/block-delta :handler (fn []
                                    (set lifecycle.delta (+ lifecycle.delta 1))
                                    nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/block-end :handler (fn []
                                    (set lifecycle.block_end
                                         (+ lifecycle.block_end 1))
                                    nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/response-end :handler (fn []
                                    (set lifecycle.finish
                                         (+ lifecycle.finish 1))
                                    nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :agent/completed :handler (fn [db]
                                    (assert (and (= db.agent.usage.input_tokens
                                                    2)
                                                 (= db.agent.usage.output_tokens
                                                    3))
                                            "stream usage was not finalized")
                                    (local content
                                           (. db.agent.messages 2 :content))
                                    (assert (and (= (. content 1 :type) :text)
                                                 (= (. content 1 :text)
                                                    "stream ")))
                                    (assert (and (= (. content 2 :type)
                                                    :thinking)
                                                 (= (. content 2 :text)
                                                    :private)))
                                    (assert (and (= (. content 3 :type) :text)
                                                 (= (. content 3 :text) :works)))
                                    (assert (and (and (and (and (= lifecycle.start
                                                                   1)
                                                                (= lifecycle.block_start
                                                                   3))
                                                           (= lifecycle.delta 3))
                                                      (= lifecycle.block_end 3))
                                                 (= lifecycle.finish 1))
                                            "stable response/block lifecycle was not emitted")
                                    (local response (. db.messages.responses 2))
                                    (assert (and (and (= response.status
                                                         :complete)
                                                      (= response.block_count 3))
                                                 (= (type response.started_wall_ms)
                                                    :number))
                                            "message response metadata is incomplete")
                                    (assert (and (= (. db.messages.blocks 2
                                                       :text)
                                                    "stream ")
                                                 (= (. db.messages.blocks 2
                                                       :chunks)
                                                    nil))
                                            "stream chunks were not compacted once")
                                    nil)}})
          nil
          (definitions :tests.integration.fixtures.stream-check declarations {}))
