(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [] (error :exploded) nil)}})
          nil
          (definitions.build :tests.integration.fixtures.fail declarations {}))
