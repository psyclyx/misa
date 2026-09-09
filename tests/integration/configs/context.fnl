(local standard (require :misa.standard))

(standard.application
  {:config {"value" 42}
   :modules {
    "module-1" {:priority 0 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/context.fnl")}}})
