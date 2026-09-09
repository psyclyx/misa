(local definitions (require :tests.declarations))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:type :app/quit}
                                          {:type :not/native}]})}})
          nil
          (definitions.collect :tests.integration.fixtures.late-effect declarations {}))
