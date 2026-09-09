;; Source-range metadata must preserve painted output. Adapted from the public
;; markdown-layout A/B harness; compilation/oracle checks are outside timings.
(local fennel (require :fennel))
(local setup (fennel.dofile :benchmarks/setup.fnl))
(global misa {:json-null {}})
(setup {:services {:json (require :misa.json)
                   :layout (require :misa.ui.layout)}})
(fn load [parser component]
  {:parser (fennel.dofile parser) :view (fennel.dofile component)})

(local baseline (load (assert (. arg 1)) (assert (. arg 2))))
(local candidate (load :extensions/misa/markdown/init.fnl
                       :extensions/misa/markdown/render.fnl))
(local source (string.rep "# Heading\n\nordinary **bold** and [link](https://example.test) with escaped \\* text\n  continuing on another source line\n\n- first second third fourth\n- next item\n\n| key | value |\n| --- | --- |\n| one | **two** |\n\n"
                          30))
(local streaming [])
(local redraw [])
(for [n 128 (+ (length source) 127) 128]
  (table.insert streaming (source:sub 1 n)))
(for [_ 1 100] (table.insert redraw source))
(print (string.format "workload,bytes=%d,stream_updates=%d,redraw_updates=%d"
                      (length source) (length streaming) (length redraw)))
(fn painted [lines]
  (icollect [_ line (ipairs lines)]
    (let [result []]
      (each [_ span (ipairs line.spans)]
        (local style (misa.json.encode (or span.style :plain)))
        (local previous (. result (length result)))
        (if (and previous (= previous.style style) (= previous.link span.link))
            (set previous.text (.. previous.text span.text))
            (table.insert result {:text span.text : style :link span.link})))
      result)))

(fn run [api inputs oracle]
  (set misa.markdown api.parser)
  (var projection nil)
  (local result [])
  (var elapsed 0)
  (each [index text (ipairs inputs)]
    (local start (os.clock))
    (set projection (api.view.project text {:columns 60 :base :assistant}
                                      projection))
    (set elapsed (+ elapsed (- (os.clock) start)))
    (local encoded (misa.json.encode (painted projection.lines)))
    (when oracle
      (assert (= encoded (. oracle index))
              (.. "painted layout differs at input " index)))
    (table.insert result encoded))
  (values elapsed result))

(each [_ workload (ipairs [{:name :stream :inputs streaming}
                           {:name :redraw :inputs redraw}])]
  (local (_ oracle) (run baseline workload.inputs))
  (for [_ 1 6] (run baseline workload.inputs oracle))
  (for [_ 1 6] (run candidate workload.inputs oracle))
  (local times {:baseline [] :candidate []})
  (for [sample 1 10]
    (each [_ name (ipairs (if (= (% sample 2) 0) [:candidate :baseline]
                              [:baseline :candidate]))]
      (collectgarbage :collect)
      (local elapsed (run (if (= name :baseline) baseline candidate)
                          workload.inputs oracle))
      (table.insert (. times name) elapsed)
      (print (string.format "sample,%s,%s,%d,%.9f" workload.name name sample
                            elapsed))))
  (each [name samples (pairs times)]
    (table.sort samples)
    (print (string.format "summary,%s,%s,median_s=%.9f,best_s=%.9f"
                          workload.name name
                          (/ (+ (. samples 5) (. samples 6)) 2) (. samples 1)))))
