(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {
                                     :fx [{:event {:last_usage {:input_tokens 7
                                                                :output_tokens 2}
                                                   :type :agent/usage
                                                   :usage {:input_tokens 9
                                                           :output_tokens 3}}
                                           :type :dispatch}
                                          {:event {:type :agent/reset}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :agent/status :handler (fn [db event]
                                    (if (= event.last_usage nil) nil
                                        (do
                                          (assert (= (next db.agent.last_usage)
                                                     nil)
                                                  "agent last usage survived clear")
                                          (assert (= (next db.usage.last_request)
                                                     nil)
                                                  "status context survived clear")
                                          {
                                           :fx [{:lines [{:spans [{:style {:foreground :default}
                                                                   :text :cleared}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))}})
          nil
          (definitions.build :tests.integration.fixtures.clear-state declarations {}))
