(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.markdown))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.selection.document))
  (app.include (. (require :tests.stock) :misa.selection.render))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/selection_document.fnl") {: config}))
  {: config :definitions app.definitions})
