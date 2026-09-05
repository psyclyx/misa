{:setup (fn []
          (misa.reg_model {:id :alpha/model :model :model :provider :alpha})
          (misa.reg_model {:id :beta/model :model :model :provider :beta})
          (misa.reg_tool {:description "switch providers"
                          :effect :tool.switch
                          :input_schema {:type :object}
                          :name :switch})
          (misa.reg_fx :provider.alpha
                       (fn [effect]
                         [{:event {:id effect.id :type :agent/stream-start}
                           :type :dispatch}
                          {:event {:delta {:arguments_json_delta "{\"value\":7,\"nested\":{\"ok\":true}}"
                                           :id :known
                                           :index 0
                                           :name :switch
                                           :type :tool_call}
                                   :id effect.id
                                   :type :agent/stream-delta}
                           :type :dispatch}
                          {:event {:id effect.id :type :agent/stream-end}
                           :type :dispatch}]))
          (misa.reg_fx :tool.switch
                       (fn [effect]
                         (assert (and (= effect.arguments.value 7)
                                      (= effect.arguments.nested.ok true)))
                         {:event {:type :cross/switch} :type :dispatch}))
          (misa.reg_event :cross/switch
                          (fn []
                            {:fx [{:event {:id :beta/model :type :model/select}
                                   :type :dispatch}
                                  {:event {:text :switched
                                           :tool_call_id :known
                                           :type :tool/result}
                                   :type :dispatch}]}))
          (var beta-calls 0)
          (misa.reg_fx :provider.beta
                       (fn [effect]
                         (set beta-calls (+ beta-calls 1))
                         (local known (. effect.messages 2 :content 1))
                         (assert (and (= (type known.arguments) :table)
                                      (= known.arguments_json nil)))
                         (assert (= (misa.json.encode known.arguments)
                                    "{\"nested\":{\"ok\":true},\"value\":7}"))
                         (local openai
                                (misa.protocols.serialize_openai_messages effect.messages))
                         (local anthropic
                                (misa.protocols.serialize_anthropic_messages effect.messages))
                         (assert (= (. openai 2 :tool_calls 1 :function
                                       :arguments)
                                    "{\"nested\":{\"ok\":true},\"value\":7}"))
                         (assert (= (. anthropic 2 :content 1 :input :nested
                                       :ok)
                                    true))
                         (if (= beta-calls 1)
                             {:event {:content [{:arguments_json "{\"portable\":true}"
                                                 :id :unknown
                                                 :name :missing
                                                 :type :tool_call}]
                                      :id effect.id
                                      :type :agent/result}
                              :type :dispatch}
                             (do
                               (local unknown (. effect.messages 4 :content 1))
                               (assert (and (and (= (type unknown.arguments)
                                                    :table)
                                                 (= unknown.arguments.portable
                                                    true))
                                            (= unknown.arguments_json nil)))
                               (assert (= (. openai 4 :tool_calls 1 :function
                                             :arguments)
                                          "{\"portable\":true}"))
                               (assert (= (. anthropic 4 :content 1 :input
                                             :portable)
                                          true))
                               {:event {:content [{:text "portable replay"
                                                   :type :text}]
                                        :id effect.id
                                        :type :agent/result}
                                :type :dispatch}))))
          nil)}

