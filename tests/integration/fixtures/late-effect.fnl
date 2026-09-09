(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:type :app/quit}
                                          {:type :not/native}]})}})
          nil
          (definitions :tests.integration.fixtures.late-effect declarations {}))
