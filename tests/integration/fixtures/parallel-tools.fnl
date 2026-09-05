{:setup (fn []
          (local setup-fx [])
          (each [_ name (ipairs [:slow :fast])]
            (table.insert setup-fx
                          {:type :register/tool
                           :value {:description (.. name " description")
                                   :effect :parallel/run
                                   :input_schema {:type :object}
                                   : name}}))
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :parallel/model
                                 :model :model
                                 :provider :parallel}})
          (var provider-calls 0)
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.parallel
                         :handler (fn [effect]
                                    (set provider-calls (+ provider-calls 1))
                                    (if (= provider-calls 1)
                                        {:event {:content [{:arguments {}
                                                            :id :first
                                                            :name :slow
                                                            :type :tool_call}
                                                           {:arguments {}
                                                            :id :second
                                                            :name :fast
                                                            :type :tool_call}]
                                                 :id effect.id
                                                 :type :agent/result}
                                         :type :dispatch}
                                        (do
                                          (assert (= provider-calls 2)
                                                  "parallel tools continued more than once")
                                          (assert (and (and (= (length effect.messages)
                                                               4)
                                                            (= (. effect.messages
                                                                  3
                                                                  :tool_call_id)
                                                               :first))
                                                       (= (. effect.messages 4
                                                             :tool_call_id)
                                                          :second))
                                                  "provider history followed completion order instead of assistant call order")
                                          {:event {:content [{:text "parallel ordered"
                                                              :type :text}]
                                                   :id effect.id
                                                   :type :agent/result}
                                           :type :dispatch})))})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :parallel/run
                         :handler (fn [effect]
                                    {:completion :parallel/done
                                     :id effect.tool_call_id
                                     :interval_ms (or (and (= effect.name :fast)
                                                           10)
                                                      50)
                                     :type :timer/start})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :parallel/done
                         :handler (fn [_ event]
                                    {:fx [{:id event.id :type :timer/stop}
                                          {:event {:text event.id
                                                   :tool_call_id event.id
                                                   :type :tool/result}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/tool-result
                         :handler (fn [db event]
                                    (when (= event.id :second)
                                      (local (first second)
                                             (values (. db.messages.blocks 2)
                                                     (. db.messages.blocks 3)))
                                      (assert (and (and (and (= first.call_id
                                                                :first)
                                                             (= first.status
                                                                :pending))
                                                        (= second.call_id
                                                           :second))
                                                   (= second.status :success))
                                              "parallel transcript sections did not update independently"))
                                    nil)})
          nil
          {:fx setup-fx})}
