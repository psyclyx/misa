(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.markdown))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/markdown.fnl") {:config config}))
  {:config config :definitions app.definitions})
