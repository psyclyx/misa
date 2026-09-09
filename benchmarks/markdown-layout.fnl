;; Compare public incremental layout APIs, excluding compilation and output checks.
(local fennel (require :fennel))
(local setup (fennel.dofile :benchmarks/setup.fnl))
(global misa {:json_null {}})
(each [_ name (ipairs [:misa.json :misa.ui.layout :misa.markdown])]
  (setup (require name)))
(local encode misa.json.encode)
(local baseline-path (assert (. arg 1) "baseline component source required"))
(setup (fennel.dofile baseline-path))
(local baseline misa.markdown.view)
(setup (fennel.dofile :extensions/misa/markdown/render.fnl))
(local candidate misa.markdown.view)
(local source (string.rep "# Heading\n\nordinary **bold** and [link](https://example.test)\n\n```lua\nlocal x = 1\n```\n\n" 40))
(local streaming [])
(local redraw [])
(for [n 128 (+ (length source) 127) 128] (table.insert streaming (source:sub 1 n)))
(for [_ 1 100] (table.insert redraw source))
(local cases [{:name :stream :inputs streaming} {:name :redraw :inputs redraw}])
(fn run [api inputs oracle]
  (var projection nil)
  (local view (and api.new-document (api.new-document)))
  (local result [])
  (var elapsed 0)
  (each [index text (ipairs inputs)]
    (local start (os.clock))
    (local lines
           (if view (view:render text {:columns 60 :base :assistant})
               (do (set projection (api.project text {:columns 60 :base :assistant} projection))
                   projection.lines)))
    (set elapsed (+ elapsed (- (os.clock) start)))
    (local encoded (encode lines))
    (when oracle (assert (= encoded (. oracle index)) "layout output differs"))
    (tset result index encoded))
  (values elapsed result))
(each [_ workload (ipairs cases)]
  (local (_ oracle) (run baseline workload.inputs))
  (for [_ 1 5] (run baseline workload.inputs oracle))
  (for [_ 1 6] (run candidate workload.inputs oracle))
  (local times {:baseline [] :candidate []})
  (for [sample 1 10]
    (each [_ name (ipairs (if (= (% sample 2) 0) [:candidate :baseline] [:baseline :candidate]))]
      (collectgarbage :collect)
      (local elapsed (run (if (= name :baseline) baseline candidate) workload.inputs oracle))
      (table.insert (. times name) elapsed)
      (print (string.format "sample,%s,%s,%d,%.9f" workload.name name sample elapsed))))
  (each [name samples (pairs times)]
    (table.sort samples)
    (print (string.format "summary,%s,%s,median_s=%.9f,best_s=%.9f" workload.name name
                          (/ (+ (. samples 5) (. samples 6)) 2) (. samples 1)))))
