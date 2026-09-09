(local standard (require :misa.standard))

(standard.application
  {:config {"themes" {"persist" false} "components" {"persist" false} "history" {"persist" false} "clipboard" {} "messages" {"max_string" 20000}}
   :modules {
    "module-1" {:priority 0 :build (require "commands")}
    "module-2" {:priority 1000 :build (require "choices")}
    "module-3" {:priority 2000 :build (require "models")}
    "module-4" {:priority 3000 :build (require "costs")}
    "module-5" {:priority 4000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/costs.fnl")}}})
