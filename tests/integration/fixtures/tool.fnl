{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/tool
                         :value {:description "Echo text"
                                 :effect :tool.echo
                                 :input_schema {:type :object}
                                 :name :echo}})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :tool.echo
                         :handler (fn [effect]
                                    (assert (= effect.arguments.value
                                               "from tool"))
                                    {:event {:text effect.arguments.value
                                             :tool_call_id effect.tool_call_id
                                             :type :tool/result}
                                     :type :dispatch})})
          nil
          {:fx setup-fx})}
