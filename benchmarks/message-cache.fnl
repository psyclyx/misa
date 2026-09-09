;; Raw message rendering: compare a saved component with the working tree.
(local fennel (require :fennel))
(local clock os.clock)
(local output print)
(local baseline-path (assert (. arg 1) "baseline source required"))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:config {}}))
(local context {:config {}})
(each [_ name (ipairs [:misa.json :misa.ui.layout :misa.markdown :misa.markdown.render])]
  (app.define ((. (require name) :build) context)))
(fn renderer [specs] (assert (. specs :components :default.transcript.assistant :render)))
(local baseline (renderer ((fennel.dofile baseline-path) context)))
(local specs ((. (fennel.dofile :extensions/misa/transcript/render.fnl) :build) context))
(local candidate (renderer specs))
(app.define {:subscriptions (or specs.subscriptions {})})
(app.install)
(local render-context {:columns 80 :interactive true})
(local source (string.rep "ordinary **bold** and [link](https://example.test)\n\n" 20))
(each [_ count (ipairs [1 16 300])]
  (local models [])
  (for [i 1 count]
    (table.insert models {:id (tostring i) :response_id :reply :rail :rail.assistant :text source}))
  (local oracle (misa.json.encode (baseline (. models 1) render-context)))
  (fn run [render]
    (var elapsed 0)
    (for [_ 1 3]
      (each [_ model (ipairs models)]
        (local start (clock))
        (local result (render model render-context))
        (set elapsed (+ elapsed (- (clock) start)))
        (assert (= oracle (misa.json.encode result)) "message output differs")))
    elapsed)
  (for [_ 1 6] (run baseline) (run candidate))
  (local times {:baseline [] :candidate []})
  (for [iteration 1 10]
    (each [_ mode (ipairs (if (= (% iteration 2) 0) [:candidate :baseline] [:baseline :candidate]))]
      (local elapsed (run (if (= mode :candidate) candidate baseline)))
      (table.insert (. times mode) elapsed)
      (output (string.format "sample,%d,%s,%d,%.9f" count mode iteration elapsed))))
  (each [mode samples (pairs times)]
    (table.sort samples)
    (output (string.format "summary,%d,%s,median_s=%.9f,best_s=%.9f" count mode
                            (/ (+ (. samples 5) (. samples 6)) 2) (. samples 1)))))
