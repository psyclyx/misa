(local definitions (require :tests.declarations))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:completion :state/loaded
                                           :namespace :integration
                                           :type :state/load}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :state/loaded :handler (fn [_ event]
                                    (assert (= event.namespace :integration))
                                    (if (= event.data misa.json-null)
                                        {:fx [{:data {:count 7
                                                      :nested {:ok true}}
                                               :namespace :integration
                                               :type :state/save}
                                              {:type :app/quit}]}
                                        (do
                                          (assert (and (= event.data.count 7)
                                                       (= event.data.nested.ok
                                                          true)))
                                          {:fx [{:lines [{:spans [{:text "state loaded"}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))}})
          nil
          (definitions.collect :tests.integration.fixtures.state declarations {}))
