(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/late-effect.fnl") {: config}))
  {: config :definitions app.definitions})
