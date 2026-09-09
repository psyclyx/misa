(local standard (require :misa.standard))

(standard.application
  {:config {"missing" misa.json-null}
   :modules {
    "module-1" {:priority 0 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/sandbox.fnl")}}})
