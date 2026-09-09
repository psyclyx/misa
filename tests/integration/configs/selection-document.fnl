(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.ui.themes")}
    "module-2" {:priority 1000 :build (require "misa.ui.themes.default")}
    "module-3" {:priority 2000 :build (require "misa.ui.components")}
    "module-4" {:priority 3000 :build (require "misa.ui.layout")}
    "module-5" {:priority 4000 :build (require "misa.markdown")}
    "module-6" {:priority 5000 :build (require "misa.keybindings")}
    "module-7" {:priority 6000 :build (require "misa.selection.document")}
    "module-8" {:priority 7000 :build (require "misa.selection.render")}
    "module-9" {:priority 8000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/selection_document.fnl")}}})
