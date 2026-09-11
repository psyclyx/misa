(let [config {:themes {:persist false}
              :components {:persist false}
              :history {:persist false}
              :clipboard {}
              :messages {:max_string 20000}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.clipboard))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.markdown))
  (app.include (. (require :tests.stock) :misa.selection.document))
  (app.include (. (require :tests.stock) :misa.selection))
  (app.include (. (require :tests.stock) :misa.markdown.render))
  (app.include (. (require :tests.stock) :misa.ui.components.group))
  (app.include (. (require :tests.stock) :misa.transcript.render))
  (app.include (. (require :tests.stock) :misa.ui.components.content))
  (app.include (. (require :tests.stock) :misa.ui.components.truncation))
  (app.include (. (require :tests.stock) :misa.transcript.tools))
  (app.include (. (require :tests.stock) :misa.transcript.tools.render))
  (app.include (. (require :tests.stock) :misa.selection.render))
  (app.include (. (require :tests.stock) :misa.editor.images.render))
  (app.include (. (require :tests.stock) :misa.editor.attachments))
  (app.include (. (require :tests.stock) :misa.transcript))
  (app.include (. (require :tests.stock) :misa.models))
  (app.include (. (require :tests.stock) :misa.costs))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/transcript-interaction.fnl") {: config}))
  (app.include (. (require :tests.stock) :misa.transcript.groups))
  {: config :definitions app.definitions})
