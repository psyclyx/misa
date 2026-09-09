(local standard (require :misa.standard))

(standard.application
  {:config {"themes" {"persist" false} "components" {"persist" false} "history" {"persist" false} "clipboard" {} "messages" {"max_string" 20000}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.ui.values")}
    "module-2" {:priority 1000 :build (require "misa.keybindings")}
    "module-3" {:priority 2000 :build (require "misa.choices")}
    "module-4" {:priority 3000 :build (require "misa.ui.layout")}
    "module-5" {:priority 4000 :build (require "misa.choices.preview")}
    "module-6" {:priority 5000 :build (require "misa.choices.layout")}
    "module-7" {:priority 6000 :build (require "misa.choices.picker")}
    "module-8" {:priority 7000 :build (require "misa.editor.history")}
    "module-9" {:priority 8000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/history.fnl")}
    :misa.models.preview {:source :misa.models.preview}}})
