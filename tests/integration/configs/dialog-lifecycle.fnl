(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.dialogs))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/dialog-lifecycle.fnl") {: config}))
  {: config :definitions app.definitions})
