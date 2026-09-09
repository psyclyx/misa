(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/constructor-trace.fnl") {:config config}))
  {:config config :definitions app.definitions})
