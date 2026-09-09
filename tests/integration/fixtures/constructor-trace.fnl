(local definitions (require :misa.definitions))

(fn []
          (local declarations [])

          (fn nested [] (error "constructor exploded") nil)

          (nested)
          nil
          (definitions.build :tests.integration.fixtures.constructor-trace declarations {}))
