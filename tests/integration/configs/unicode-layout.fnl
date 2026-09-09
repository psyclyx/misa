(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.commands.keybindings")}
    "module-2" {:priority 1000 :build (require "misa.ui.values")}
    "module-3" {:priority 2000 :build (require "misa.ui.themes")}
    "module-4" {:priority 3000 :build (require "misa.ui.themes.default")}
    "module-5" {:priority 4000 :build (require "misa.ui.components")}
    "module-6" {:priority 5000 :build (require "misa.ui.layout")}
    "module-7" {:priority 6000 :build (require "misa.choices")}
    "module-8" {:priority 7000 :build (require "misa.choices.preview")}
    "module-9" {:priority 8000 :build (require "misa.choices.layout")}
    "module-10" {:priority 9000 :build (require "misa.ui.components.editor")}
    "module-11" {:priority 10000 :build (require "misa.ui.components.picker")}
    "module-12" {:priority 11000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/unicode-layout.fnl")}}})
