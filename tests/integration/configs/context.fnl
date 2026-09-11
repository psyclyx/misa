(let [config {:value 42}
      app ((require :tests.application) {: config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/context.fnl") {: config}))
  {: config :definitions app.definitions})
