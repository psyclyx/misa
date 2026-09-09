(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.markdown))
  (app.include (. (require :tests.stock) :misa.markdown.render))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/markdown-rendering.fnl") {:config config}))
  {:config config :definitions app.definitions})
