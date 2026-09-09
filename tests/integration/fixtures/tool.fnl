(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:description "Echo text"
                                 :effect :tool.echo
                                 :input_schema {:type :object}
                                 :name :echo}] {:catalog :tools :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :effects :id :tool.echo :value (fn [effect]
                                    (assert (= effect.arguments.value
                                               "from tool"))
                                    {:event {:text effect.arguments.value
                                             :tool_call_id effect.tool_call_id
                                             :type :tool/result}
                                     :type :dispatch})})
          nil
          (definitions :tests.integration.fixtures.tool declarations {}))
