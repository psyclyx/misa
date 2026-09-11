(let [config {:animations {:persist false :enabled true}
              :themes {:persist false}
              :components {:persist false}
              :status {:indicators [:activity]}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.animations))
  (app.include (. (require :tests.stock) :misa.ui.animations.default))
  (app.include (. (require :tests.stock) :misa.ui.status.indicators))
  (app.include (. (require :tests.stock) :misa.ui.status.render))
  (app.include (. (require :tests.stock) :misa.ui.status))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/animations.fnl") {: config}))
  (app.include (. (require :tests.stock) :misa.usage))
  {: config :definitions app.definitions})
