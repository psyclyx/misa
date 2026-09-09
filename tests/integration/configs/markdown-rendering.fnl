(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.ui.themes")}
    "module-2" {:priority 1000 :build (require "misa.ui.themes.default")}
    "module-3" {:priority 2000 :build (require "misa.ui.components")}
    "module-4" {:priority 3000 :build (require "misa.ui.layout")}
    "module-5" {:priority 4000 :build (require "misa.text.markdown")}
    "module-6" {:priority 5000 :build (require "misa.ui.components.markdown")}
    "module-7" {:priority 6000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/markdown-rendering.fnl")}}})
