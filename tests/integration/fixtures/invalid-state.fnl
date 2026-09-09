(local definitions (require :tests.declarations))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:completion :invalid/loaded
                                           :namespace :integration
                                           :type :state/load}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :invalid/loaded :handler (fn [_ event]
                                    (assert (and (= event.ok false)
                                                 (= event.message :InvalidState)))
                                    {:fx [{:type :app/quit}]})}})
          nil
          (definitions.collect :tests.integration.fixtures.invalid-state declarations {}))
