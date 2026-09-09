(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.agent))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/runtime-regressions-reset.fnl") {:config config}))
  {:config config :definitions app.definitions})
