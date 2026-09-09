(local fixture-fennel (require :fennel))
(set fixture-fennel.path (.. "@ROOT@/?.fnl;" fixture-fennel.path))
(let [config {"models" {"default" "claude/claude-sonnet-5"}
              "providers" {"claude" {"executable" "@WORK@/claude"
                                     "mcp_command" "@BIN@"
                                     "mcp_arguments" ["mcp"
                                                      "--config"
                                                      "@WORK@/claude.fnl"]}}}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.agent.stream))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.providers.claude))
  (app.include (. (require :tests.stock) :misa.tools.files))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.markdown))
  (app.include (. (require :tests.stock) :misa.markdown.render))
  (app.include (. (require :tests.stock) :misa.ui.components.content))
  (app.include (. (require :tests.stock) :misa.ui.components.truncation))
  (app.include (. (require :tests.stock) :misa.transcript.tools))
  (app.include (. (require :tests.stock) :misa.transcript.tools.render))
  (app.include (. (require :tests.stock) :misa.ui.components.group))
  (app.include (. (require :tests.stock) :misa.transcript.render))
  (app.include (. (require :tests.stock) :misa.editor.render))
  (app.include (. (require :tests.stock) :misa.choices.picker.render))
  (app.include (. (require :tests.stock) :misa.ui.status.render))
  (app.include (. (require :tests.stock) :misa.ui.chrome))
  (app.include (. (require :tests.stock) :misa.transcript))
  (app.include (. (require :tests.stock) :misa.models))
  (app.include (. (require :tests.stock) :misa.agent))
  (app.include (. (require :tests.stock) :misa.commands))
  (app.include (. (require :tests.stock) :misa.choices))
  (app.include (. (require :tests.stock) :misa.editor))
  (app.include (. (require :tests.stock) :misa.ui))
  (app.include (. (require :tests.stock) :misa.transcript.groups))
  {:config config :definitions app.definitions})
