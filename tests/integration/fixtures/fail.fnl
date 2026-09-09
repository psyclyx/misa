(local definitions (require :tests.declarations))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [] (error :exploded) nil)}})
          nil
          (definitions.collect :tests.integration.fixtures.fail declarations {}))
