(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "keybindings")}
    "module-2" {:priority 1000 :build (require "values")}
    "module-3" {:priority 2000 :build (require "themes")}
    "module-4" {:priority 3000 :build (require "theme.default")}
    "module-5" {:priority 4000 :build (require "components")}
    "module-6" {:priority 5000 :build (require "layout")}
    "module-7" {:priority 6000 :build (require "choices")}
    "module-8" {:priority 7000 :build (require "choices.preview")}
    "module-9" {:priority 8000 :build (require "choices.layout")}
    "module-10" {:priority 9000 :build (require "component.editor")}
    "module-11" {:priority 10000 :build (require "component.picker")}
    "module-12" {:priority 11000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/unicode-layout.fnl")}}})
