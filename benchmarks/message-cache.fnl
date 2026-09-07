;; Raw message rendering: compare a saved component with the working tree.
(local fennel (require :fennel))
(local clock os.clock)
(local output print)
(local baseline-path (assert (. arg 1) "baseline source required"))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:config {}})
(each [_ name (ipairs [:json :layout :markdown :component/markdown])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))
(fn renderer [specs]
  (var result nil)
  (each [_ spec (ipairs specs.fx)]
    (when (= spec.id :default.transcript.assistant) (set result spec.value.render)))
  (assert result))
(local baseline (renderer ((. (fennel.dofile baseline-path) :setup) context)))
(local specs ((. (fennel.dofile :extensions/component/message.fnl) :setup) context))
(local candidate (renderer specs))
;; Only the component's derived subscriptions need registration in this probe.
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/sub) (misa._setup_effects {:fx [spec]})))
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
