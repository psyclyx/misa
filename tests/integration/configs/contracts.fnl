(let [config {"nested" {"value" 7}}
      app ((require :tests.application) {:config config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/contracts.fnl") {:config config}))
  {:config config :definitions app.definitions})
