(let [config {:models {:default :fake/default}
              :providers {:fake {:responses [{:stream [{:type :tool_call
                                                        :id :invalid
                                                        :name :shell
                                                        :arguments {}}]}
                                             :recovered]}}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.agent.stream))
  (app.include (. (require :tests.stock) :misa.json))
  (app.include (. (require :tests.stock) :misa.providers.fake))
  (app.include (. (require :tests.stock) :misa.tools.shell))
  (app.include (. (require :tests.stock) :misa.models))
  (app.include (. (require :tests.stock) :misa.agent))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/runtime-regressions-invalid.fnl") {: config}))
  {: config :definitions app.definitions})
