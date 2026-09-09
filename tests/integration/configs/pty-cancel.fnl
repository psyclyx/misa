(local standard (require :misa.standard))

(standard.application
  {:config {"cancel" true "pid" "@WORK@/child.pid"}
   :modules {
    "module-1" {:priority 0 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/pty.fnl")}}})
