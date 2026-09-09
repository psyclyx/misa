(local standard (require :misa.standard))

(standard.application
  {:config {"themes" {"persist" false} "components" {"persist" false} "history" {"persist" false} "clipboard" {} "messages" {"max_string" 20000}}
   :modules {
    "module-1" {:priority 0 :build (require "values")}
    "module-2" {:priority 1000 :build (require "keybindings")}
    "module-3" {:priority 2000 :build (require "choices")}
    "module-4" {:priority 3000 :build (require "layout")}
    "module-5" {:priority 4000 :build (require "choices.preview")}
    "module-6" {:priority 5000 :build (require "choices.layout")}
    "module-7" {:priority 6000 :build (require "picker")}
    "module-8" {:priority 7000 :build (require "history")}
    "module-9" {:priority 8000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/history.fnl")}}})
