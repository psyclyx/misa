(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/conversation-read.fnl") {:config config}))
  {:config config :definitions app.definitions})
