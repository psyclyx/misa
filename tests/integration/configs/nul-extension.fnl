(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (((. (require :fennel) :dofile) "bad\000.fnl") {:config config}))
  {:config config :definitions app.definitions})
