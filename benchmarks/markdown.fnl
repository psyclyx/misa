(local fennel (require :fennel))
(local lua-dofile dofile)
(fn dofile [path]
  (if (path:match "%.fnl$") (fennel.dofile path) (lua-dofile path)))

;; Public parser A/B harness. Run sequentially in a quiet environment:

;; tools/fennel benchmarks/markdown.fnl /path/to/baseline.lua extensions/misa/markdown/init.fnl

(local baseline-path (assert (. arg 1) "baseline source required"))

(local candidate-path (assert (. arg 2) "candidate source required"))

(local size (or (tonumber (. arg 3)) 16384))

(local samples (or (tonumber (. arg 4)) 10))

(global misa {:json-null {}})

(set misa.json (dofile :extensions/misa/json.fnl))

(local encode misa.json.encode)

(fn load [path]
  (. (dofile path) :parse))

(local (baseline candidate) (values (load baseline-path) (load candidate-path)))

(local workloads [{:name :plain :text (string.rep :x size)}
                  {:name :mixed
                   :text (string.rep "ordinary **bold** and [link](https://example.test) with `code`

" (math.floor (/ size 64)))}])

(fn time [parser text]
  (collectgarbage :collect)
  (local started (os.clock))
  (local result (parser text))
  (local elapsed (- (os.clock) started))
  (values elapsed (encode result)))

(each [_ workload (ipairs workloads)]
  (var (distinct oracle) (values {} nil))
  (for [_ 1 6] (local (_ value) (time baseline workload.text))
    (tset distinct value true)
    (set oracle value))
  (var count 0)
  (each [_ (pairs distinct)] (set count (+ count 1)))
  (assert (= count 1) "nondeterministic baseline")
  (local times {:baseline {} :candidate {}})
  (for [sample 1 samples]
    (local order (or (and (= (% sample 2) 0) [:candidate :baseline])
                     [:baseline :candidate]))
    (each [_ name (ipairs order)]
      (local (elapsed value) (time (or (and (= name :baseline) baseline)
                                       candidate)
                                   workload.text))
      (assert (= value oracle) (.. "AST mismatch for " workload.name " " name))
      (tset (. times name) (+ (length (. times name)) 1) elapsed)
      (print (string.format "sample,%s,%s,%d,%.9f" workload.name name sample
                            elapsed))))
  (each [_ name (ipairs [:baseline :candidate])]
    (table.sort (. times name))
    (local n (length (. times name)))
    (local median (/ (+ (. times name (math.floor (/ (+ n 1) 2)))
                        (. times name (math.ceil (/ (+ n 1) 2))))
                     2))
    (print (string.format "summary,%s,%s,bytes=%d,n=%d,median_s=%.9f,best_s=%.9f,oracle_distinct=%d"
                          workload.name name (length workload.text) n median
                          (. times name 1) count))))
