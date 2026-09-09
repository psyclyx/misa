(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "themes")}
    "module-2" {:priority 1000 :build (require "theme.default")}
    "module-3" {:priority 2000 :build (require "components")}
    "module-4" {:priority 3000 :build (require "layout")}
    "module-5" {:priority 4000 :build (require "markdown")}
    "module-6" {:priority 5000 :build (require "component.markdown")}
    "module-7" {:priority 6000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/markdown-rendering.fnl")}}})
