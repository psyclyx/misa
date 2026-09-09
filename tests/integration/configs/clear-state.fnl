(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.ui.status")}
    "module-2" {:priority 1000 :build (require "misa.ui.transcript")}
    "module-3" {:priority 2000 :build (require "misa.agent")}
    "module-4" {:priority 3000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/clear-state.fnl")}}})
