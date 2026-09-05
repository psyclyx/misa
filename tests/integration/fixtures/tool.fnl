{:setup (fn []
          (misa.reg_tool {:description "Echo text"
                          :effect :tool.echo
                          :input_schema {:type :object}
                          :name :echo})
          (misa.reg_fx :tool.echo
                       (fn [effect]
                         (assert (= effect.arguments.value "from tool"))
                         {:event {:text effect.arguments.value
                                  :tool_call_id effect.tool_call_id
                                  :type :tool/result}
                          :type :dispatch}))
          nil)}

