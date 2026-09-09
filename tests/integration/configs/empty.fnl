(let [config {}
      app ((require :tests.application) {:config config})]
  {:config config :definitions app.definitions})
