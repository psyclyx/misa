(let [config {:themes {:default :test}
              :animations {:default :pulse}
              :components {:roles {:test.role :test.first}}}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.animations))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/semantic-components.fnl") {: config}))
  {: config :definitions app.definitions})
