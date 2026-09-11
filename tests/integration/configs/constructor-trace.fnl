(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/constructor-trace.fnl") {: config}))
  {: config :definitions app.definitions})
