(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/pty.fnl") {: config}))
  {: config :definitions app.definitions})
