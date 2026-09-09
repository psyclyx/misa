(let [config {}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.tools.shell))
  {:config config :definitions app.definitions})
