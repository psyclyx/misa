{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :alpha/model
                                 :model :model
                                 :provider :alpha}})
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :beta/model :model :model :provider :beta}})
          (table.insert setup-fx
                        {:type :register/tool
                         :value {:description "switch providers"
                                 :effect :tool.switch
                                 :input_schema {:type :object}
                                 :name :switch}})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.alpha
                         :handler (fn [effect]
                                    [{:event {:id effect.id
                                              :type :agent/stream-start}
                                      :type :dispatch}
                                     {:event {:delta {:arguments_json_delta "{\"value\":7,\"nested\":{\"ok\":true}}"
                                                      :id :known
                                                      :index 0
                                                      :name :switch
                                                      :type :tool_call}
                                              :id effect.id
                                              :type :agent/stream-delta}
                                      :type :dispatch}
                                     {:event {:id effect.id
                                              :type :agent/stream-end}
                                      :type :dispatch}])})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :tool.switch
                         :handler (fn [effect]
                                    (assert (and (= effect.arguments.value 7)
                                                 (= effect.arguments.nested.ok
                                                    true)))
                                    {:event {:type :cross/switch}
                                     :type :dispatch})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :cross/switch
                         :handler (fn []
                                    {:fx [{:event {:id :beta/model
                                                   :type :model/select}
                                           :type :dispatch}
                                          {:event {:text :switched
                                                   :tool_call_id :known
                                                   :type :tool/result}
                                           :type :dispatch}]})})
          (var beta-calls 0)
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.beta
                         :handler (fn [effect]
                                    (set beta-calls (+ beta-calls 1))
                                    (local known
                                           (. effect.messages 2 :content 1))
                                    (assert (and (= (type known.arguments)
                                                    :table)
                                                 (= known.arguments_json nil)))
                                    (assert (= (misa.json.encode known.arguments)
                                               "{\"nested\":{\"ok\":true},\"value\":7}"))
                                    (local openai
                                           (misa.protocols.serialize_openai_messages effect.messages))
                                    (local anthropic
                                           (misa.protocols.serialize_anthropic_messages effect.messages))
                                    (assert (= (. openai 2 :tool_calls 1
                                                  :function :arguments)
                                               "{\"nested\":{\"ok\":true},\"value\":7}"))
                                    (assert (= (. anthropic 2 :content 1 :input
                                                  :nested :ok)
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
                                          (local unknown
                                                 (. effect.messages 4 :content
                                                    1))
                                          (assert (and (and (= (type unknown.arguments)
                                                               :table)
                                                            (= unknown.arguments.portable
                                                               true))
                                                       (= unknown.arguments_json
                                                          nil)))
                                          (assert (= (. openai 4 :tool_calls 1
                                                        :function :arguments)
                                                     "{\"portable\":true}"))
                                          (assert (= (. anthropic 4 :content 1
                                                        :input :portable)
                                                     true))
                                          {:event {:content [{:text "portable replay"
                                                              :type :text}]
                                                   :id effect.id
                                                   :type :agent/result}
                                           :type :dispatch})))})
          nil
          {:fx setup-fx})}
