(local definitions (require :tests.declarations))

(fn []
          (local declarations [])

          (fn nested [] (error "constructor exploded") nil)

          (nested)
          nil
          (definitions.collect :tests.integration.fixtures.constructor-trace declarations {}))
