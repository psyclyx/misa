(local standard (require :misa.standard))

(standard.application
  {:config {"themes" {"default" "test"} "animations" {"default" "pulse"} "components" {"roles" {"test.role" "test.first"}}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.ui.themes")}
    "module-2" {:priority 1000 :build (require "misa.ui.animations")}
    "module-3" {:priority 2000 :build (require "misa.ui.components")}
    "module-4" {:priority 3000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/semantic-components.fnl")}}})
