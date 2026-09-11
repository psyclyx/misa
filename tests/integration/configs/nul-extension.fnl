(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (((. (require :fennel) :dofile) "bad\000.fnl") {: config}))
  {: config :definitions app.definitions})
