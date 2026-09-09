(local standard (require :misa.standard))

(standard.application
  {:config {"components" {"persist" false}}
   :modules {
    "module-1" {:priority 0 :build (require "keybindings")}
    "module-2" {:priority 1000 :build (require "values")}
    "module-3" {:priority 2000 :build (require "themes")}
    "module-4" {:priority 3000 :build (require "theme.default")}
    "module-5" {:priority 4000 :build (require "components")}
    "module-6" {:priority 5000 :build (require "component.status")}
    "module-7" {:priority 6000 :build (require "component.chrome")}
    "module-8" {:priority 7000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/visual-swap.fnl")}}})
