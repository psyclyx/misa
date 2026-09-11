(let [config {:choices {:purposes {:models [:frecency :all]}}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.choices.matching))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.commands))
  (app.include (. (require :tests.stock) :misa.choices))
  (app.include (. (require :tests.stock) :misa.choices.preview))
  (app.include (. (require :tests.stock) :misa.choices.layout))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/choice-contracts.fnl") {: config}))
  (app.include (. (require :tests.stock) :misa.models.preview))
  (tset app.definitions.keybindings :choices/open_overlay
        {:context :choices :action :open_overlay :default [:alt+x]})
  (tset app.definitions.keybindings :choices/option_1_1
        {:context :choices :action :option_1_1 :default [:alt+z]})
  {: config :definitions app.definitions})
