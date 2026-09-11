(let [config {:components {:persist false}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.status.render))
  (app.include (. (require :tests.stock) :misa.ui.chrome))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/visual-swap.fnl") {: config}))
  {: config :definitions app.definitions})
