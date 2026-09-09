(local standard (require :misa.standard))

(standard.application
  {:config {"components" {"persist" false}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.commands.keybindings")}
    "module-2" {:priority 1000 :build (require "misa.ui.values")}
    "module-3" {:priority 2000 :build (require "misa.ui.themes")}
    "module-4" {:priority 3000 :build (require "misa.ui.themes.default")}
    "module-5" {:priority 4000 :build (require "misa.ui.components")}
    "module-6" {:priority 5000 :build (require "misa.ui.components.status")}
    "module-7" {:priority 6000 :build (require "misa.ui.components.chrome")}
    "module-8" {:priority 7000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/visual-swap.fnl")}}})
