(let [config {}
      app ((require :tests.application) {: config})]
  {: config :definitions app.definitions})
