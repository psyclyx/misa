(let [config {"cancel" true "pid" "@WORK@/child.pid"}
      app ((require :tests.application) {:config config})]
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/pty.fnl") {:config config}))
  {:config config :definitions app.definitions})
