(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn []
                                    {:fx [{:entries [{:data {:content [{:text :hello
                                                                        :type :text}]
                                                             :role :user}
                                                      :kind :message}
                                                     {:data {:content [{:text :hi
                                                                        :type :text}]
                                                             :role :assistant}
                                                      :kind :message}]
                                           :completion :conversation/appended
                                           :conversation :integration
                                           :id :append-1
                                           :type :conversation/append}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :conversation/appended
                         :handler (fn [_ event]
                                    (assert (= event.ok true))
                                    (assert (= event.data.count 2))
                                    {:fx [{:lines [{:spans [{:text (.. "appended 2 at "
                                                                       (tostring event.data.last_seq))}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.conversation declarations {}))
