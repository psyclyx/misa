(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.dialogs))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/protected-dialog.fnl") {:config config}))
  {:config config :definitions app.definitions})
