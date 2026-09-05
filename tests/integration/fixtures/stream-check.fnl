{:setup (fn []
          (local lifecycle {:block_end 0
                            :block_start 0
                            :delta 0
                            :finish 0
                            :start 0})
          (misa.reg_event :transcript/response-start
                          (fn [] (set lifecycle.start (+ lifecycle.start 1))
                            nil))
          (misa.reg_event :transcript/block-start
                          (fn []
                            (set lifecycle.block_start
                                 (+ lifecycle.block_start 1))
                            nil))
          (misa.reg_event :transcript/block-delta
                          (fn [] (set lifecycle.delta (+ lifecycle.delta 1))
                            nil))
          (misa.reg_event :transcript/block-end
                          (fn []
                            (set lifecycle.block_end (+ lifecycle.block_end 1))
                            nil))
          (misa.reg_event :transcript/response-end
                          (fn [] (set lifecycle.finish (+ lifecycle.finish 1))
                            nil))
          (misa.reg_event :agent/completed
                          (fn [db]
                            (assert (and (= db.agent.usage.input_tokens 2)
                                         (= db.agent.usage.output_tokens 3))
                                    "stream usage was not finalized")
                            (local content (. db.agent.messages 2 :content))
                            (assert (and (= (. content 1 :type) :text)
                                         (= (. content 1 :text) "stream ")))
                            (assert (and (= (. content 2 :type) :thinking)
                                         (= (. content 2 :text) :private)))
                            (assert (and (= (. content 3 :type) :text)
                                         (= (. content 3 :text) :works)))
                            (assert (and (and (and (and (= lifecycle.start 1)
                                                        (= lifecycle.block_start
                                                           3))
                                                   (= lifecycle.delta 3))
                                              (= lifecycle.block_end 3))
                                         (= lifecycle.finish 1))
                                    "stable response/block lifecycle was not emitted")
                            (local response (. db.messages.responses 2))
                            (assert (and (and (= response.status :complete)
                                              (= response.block_count 3))
                                         (= (type response.started_wall_ms)
                                            :number))
                                    "message response metadata is incomplete")
                            (assert (and (= (. db.messages.blocks 2 :text)
                                            "stream ")
                                         (= (. db.messages.blocks 2 :chunks)
                                            nil))
                                    "stream chunks were not compacted once")
                            nil))
          nil)}

