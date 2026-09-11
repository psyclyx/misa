(let [config {:status {:indicators [{:id :important :priority 100}
                                    {:id :optional :hotkey true :priority 1}]}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.ui.status.indicators))
  (app.include (. (require :tests.stock) :misa.ui.status.render))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/indicators.fnl") {: config}))
  {: config :definitions app.definitions})
