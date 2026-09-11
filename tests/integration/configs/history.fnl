(let [config {:themes {:persist false}
              :components {:persist false}
              :history {:persist false}
              :clipboard {}
              :messages {:max_string 20000}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.choices))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.choices.preview))
  (app.include (. (require :tests.stock) :misa.choices.layout))
  (app.include (. (require :tests.stock) :misa.choices.picker))
  (app.include (. (require :tests.stock) :misa.editor.history))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/history.fnl") {: config}))
  (app.include (. (require :tests.stock) :misa.models.preview))
  {: config :definitions app.definitions})
