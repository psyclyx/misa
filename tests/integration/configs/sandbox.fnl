(let [config {"missing" misa.json-null}
      app ((require :tests.application) {:config config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/sandbox.fnl") {:config config}))
  {:config config :definitions app.definitions})
