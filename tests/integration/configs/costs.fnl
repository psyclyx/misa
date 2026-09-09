(let [config {"themes" {"persist" false}
              "components" {"persist" false}
              "history" {"persist" false}
              "clipboard" {}
              "messages" {"max_string" 20000}}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.commands))
  (app.include (. (require :tests.stock) :misa.choices))
  (app.include (. (require :tests.stock) :misa.models))
  (app.include (. (require :tests.stock) :misa.costs))
  (app.include (((. (require :fennel) :dofile) "@ROOT@/tests/costs.fnl") {:config config}))
  {:config config :definitions app.definitions})
