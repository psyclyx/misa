(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.ui.status))
  (app.include (. (require :tests.stock) :misa.transcript))
  (app.include (. (require :tests.stock) :misa.agent))
  (app.include (. (require :tests.stock) :misa.usage))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/clear-state.fnl") {:config config}))
  {:config config :definitions app.definitions})
