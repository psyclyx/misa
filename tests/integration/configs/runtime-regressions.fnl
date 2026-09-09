(let [config {"models" {"default" "claude/claude-sonnet-5"}
              "providers" {"claude" {"executable" "@WORK@/claude"}}}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.agent.stream))
  (app.include (. (require :tests.stock) :misa.json))
  (app.include (. (require :tests.stock) :misa.providers.claude))
  (app.include (. (require :tests.stock) :misa.tools.shell))
  (app.include (. (require :tests.stock) :misa.models))
  (app.include (. (require :tests.stock) :misa.agent))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/runtime-regressions-check.fnl") {:config config}))
  {:config config :definitions app.definitions})
