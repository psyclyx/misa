(local standard (require :misa.standard))

(standard.application
  {:config {"animations" {"persist" false "enabled" true} "themes" {"persist" false} "components" {"persist" false} "status" {"indicators" ["activity"]}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.commands.keybindings")}
    "module-2" {:priority 1000 :build (require "misa.ui.values")}
    "module-3" {:priority 2000 :build (require "misa.ui.layout")}
    "module-4" {:priority 3000 :build (require "misa.ui.themes")}
    "module-5" {:priority 4000 :build (require "misa.ui.themes.default")}
    "module-6" {:priority 5000 :build (require "misa.ui.components")}
    "module-7" {:priority 6000 :build (require "misa.ui.animations")}
    "module-8" {:priority 7000 :build (require "misa.ui.animations.default")}
    "module-9" {:priority 8000 :build (require "misa.ui.status.indicators")}
    "module-10" {:priority 9000 :build (require "misa.ui.components.status")}
    "module-11" {:priority 10000 :build (require "misa.ui.status")}
    "module-12" {:priority 11000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/animations.fnl")}}})
