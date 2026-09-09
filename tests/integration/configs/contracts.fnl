(local standard (require :misa.standard))

(standard.application
  {:config {"nested" {"value" 7}}
   :modules {
    "module-1" {:priority 0 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/contracts.fnl")}}})
