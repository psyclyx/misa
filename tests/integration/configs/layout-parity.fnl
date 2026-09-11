(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/layout-parity.fnl") {: config}))
  (app.include (. (require :tests.stock) :misa.ui.layout) {: config})
  {: config :definitions app.definitions})
