(local fennel (require :fennel))
(local setup (fennel.dofile :benchmarks/setup.fnl))
(local lua-dofile dofile)
(fn dofile [path]
  (if (path:match "%.fnl$") (fennel.dofile path) (lua-dofile path)))

;; Parser-only streaming benchmark, including unchanged animation redraws.
;; Compare incremental APIs when available; older baselines use their full parser.

;; tools/fennel benchmarks/markdown-streaming.fnl baseline.lua extensions/misa/text/markdown.fnl

(fn equal [a b]
  (when (not= (type a) (type b)) (lua "return false"))
  (when (not= (type a) :table)
    (let [___antifnl_rtn_1___ (= a b)] (lua "return ___antifnl_rtn_1___")))
  (each [k v (pairs a)]
    (when (not (equal v (. b k)))
      (lua "return false")))
  (each [k (pairs b)]
    (when (= (. a k) nil) (lua "return false")))
  true)

(fn load [path]
  (global misa {})
  (setup (dofile path) {:config {}})
  misa.markdown)

(local (baseline candidate)
       (values (load (assert (. arg 1))) (load (assert (. arg 2)))))

(io.stdout:setvbuf :line)

(local samples (or (tonumber (. arg 3)) 10))

(print (.. "baseline_api," (if baseline.new-document :incremental :full)))

(local workloads
       [{:name :paragraph :text (string.rep "ordinary words " 1100)}
        {:name :blocks
         :text (string.rep "# Section

ordinary **bold** [link](https://example.test) and `code`

" 240)}
        {:name :fence
         :text (.. "```lua\n" (string.rep "local value = 123\n" 950) "```")}
        {:name :table
         :text (.. "| Key | Value |\n| --- | --- |\n" (string.rep "| name | **value** |
" 800))}
        {:name :redraw
         :redraw true
         :text (string.rep "ordinary **bold** and `code`\n\n" 550)}])

(each [_ workload (ipairs workloads)]
  (local inputs {})
  (if workload.redraw
      (for [_ 1 128]
        (tset inputs (+ (length inputs) 1) workload.text))
      (for [size 128 (+ (length workload.text) 127) 128]
        (tset inputs (+ (length inputs) 1) (workload.text:sub 1 size))))

  (fn run [name verify]
    (var (result checksum) (values nil 0))
    (local stream
           (if (= name :candidate) (candidate.new-document)
               (and (= name :baseline) baseline.new-document) (baseline.new-document)))
    (each [i text (ipairs inputs)]
      (set result (or (and stream (stream:update text))
                      (or (and (= name :full) (candidate.parse text))
                          (baseline.parse text))))
      (when verify
        (assert (equal result (baseline.parse text))
                (.. workload.name " AST mismatch at " i)))
      (set checksum (+ checksum (length result.source) (length result.blocks))))
    (values result checksum))

  (local (oracle checksum) (run :baseline))
  (for [_ 1 6]
    (local (actual digest) (run :baseline))
    (assert (and (equal actual oracle) (= digest checksum))
            "nondeterministic baseline"))
  (run :candidate true)
  (run :full true)
  (local times {:baseline {} :candidate {} :full {}})
  (for [sample 1 samples]
    (each [_ name (ipairs (or (and (= (% sample 2) 0)
                                   [:candidate :full :baseline])
                              [:baseline :full :candidate]))]
      (collectgarbage :collect)
      (local start (os.clock))
      (local (actual digest) (run name))
      (local elapsed (- (os.clock) start))
      (assert (and (equal actual oracle) (= digest checksum))
              (.. workload.name " timed result mismatch"))
      (tset (. times name) (+ (length (. times name)) 1) elapsed)
      (print (string.format "sample,%s,%s,%d,%.9f" workload.name name sample
                            elapsed))))
  (each [_ name (ipairs [:baseline :full :candidate])]
    (table.sort (. times name))
    (local n (length (. times name)))
    (print (string.format "summary,%s,%s,bytes=%d,updates=%d,n=%d,median_s=%.9f,best_s=%.9f"
                          workload.name name (length workload.text)
                          (length inputs) n
                          (/ (+ (. times name (math.floor (/ (+ n 1) 2)))
                                (. times name (math.ceil (/ (+ n 1) 2))))
                             2) (. times name 1)))))
