(let [config {"themes" {"persist" false}
              "components" {"persist" false}
              "history" {"persist" false}
              "clipboard" {}
              "messages" {"max_string" 20000}}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.choices))
  (app.include (. (require :tests.stock) :misa.choices.preview))
  (app.include (. (require :tests.stock) :misa.choices.layout))
  (app.include (. (require :tests.stock) :misa.choices.picker.render))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/choice-compact.fnl") {:config config}))
  (app.include (. (require :tests.stock) :misa.models.preview))
  {:config config :definitions app.definitions})
