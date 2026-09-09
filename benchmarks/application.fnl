(local fennel (require :fennel))
(local standard (require :misa.standard))
(local providers [(require :misa.standard.providers.anthropic)
                  (require :misa.standard.providers.kimi)
                  (require :misa.standard.providers.openai)
                  (require :misa.standard.providers.openrouter)
                  (require :misa.standard.providers.openai-codex)
                  (require :misa.standard.providers.claude)
                  (require :misa.standard.providers.auth)])

(fn application [fixture settings]
  "Prepare stock application data with deterministic benchmark inputs."
  (let [app (misa.snapshot standard)]
    (set app.config.benchmark settings)
    (set app.config.models.default
         (if (= fixture :picker-key-repeat) :bench/model-00001 :bench/model))
    (each [_ name (ipairs [:history :themes :components :preferences])]
      (tset app.config name
            (misa.patch (or (. app.config name) {}) {:persist false})))
    (each [_ catalogs (ipairs providers)]
      (each [kind entries (pairs catalogs)]
        (each [id (pairs entries)] (tset (. app.definitions kind) id nil))))
    (let [definitions ((fennel.dofile (.. "benchmarks/" fixture ".fnl")) {:config app.config})]
      (each [kind entries (pairs definitions)]
        (when (not (. app.definitions kind)) (tset app.definitions kind {}))
        (each [id value (pairs entries)]
          (when (= kind :events) (set value.priority -1000))
          (tset (. app.definitions kind) id value))))
    app))

application
