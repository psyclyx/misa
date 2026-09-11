(let [config {}
      app ((require :tests.application) {: config})]
  (app.include (. (require :tests.stock) :misa.tools.shell))
  {: config :definitions app.definitions})
