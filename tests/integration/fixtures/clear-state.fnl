{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    {
                                     :fx [{:event {:last_usage {:input_tokens 7
                                                                :output_tokens 2}
                                                   :type :agent/usage
                                                   :usage {:input_tokens 9
                                                           :output_tokens 3}}
                                           :type :dispatch}
                                          {:event {:type :agent/reset}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/status
                         :handler (fn [db event]
                                    (if (= event.last_usage nil) nil
                                        (do
                                          (assert (= (next db.agent.last_usage)
                                                     nil)
                                                  "agent last usage survived clear")
                                          (assert (= (next db.status.last_usage)
                                                     nil)
                                                  "status context survived clear")
                                          {
                                           :fx [{:lines [{:spans [{:style {:foreground :default}
                                                                   :text :cleared}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))})
          nil
          {:fx setup-fx})}
