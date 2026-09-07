;; Compare replacement materialization with a saved state.fnl implementation.
(local fennel (require :fennel))
(local clock os.clock)
(local baseline-path (assert (. arg 1) "baseline source required"))
(local baseline ((fennel.dofile baseline-path) {}))
(local candidate ((fennel.dofile :src/lua_runtime/state.fnl) {}))
(fn equal [a b]
  (if (not= (type a) (type b)) false
      (not= (type a) :table) (= a b)
      (do
        (each [k v (pairs a)] (when (not (equal v (. b k))) (lua "return false")))
        (each [k (pairs b)] (when (= (. a k) nil) (lua "return false")))
        true)))
(each [_ count (ipairs [1 16 300])]
  (local old {:items (fcollect [i 1 count] {:id i :text :before :nested {:keep true}})})
  (each [_ workload (ipairs [:unchanged :tail :all :insert])]
    (local items (fcollect [i 1 count]
                  (if (or (= workload :all) (and (= workload :tail) (= i count)))
                      {:id i :text :after :nested (. old.items i :nested)}
                      (. old.items i))))
    (local source (if (= workload :insert) {} old))
    (local patches {:baseline {:items (baseline.replace items)}
                   :candidate {:items (candidate.replace items)}})
    (local expected (baseline.patch source patches.baseline))
    (fn run [api patch]
      (local start (clock))
      (var result nil)
      (for [_ 1 64] (set result (api.patch source patch)))
      (local elapsed (- (clock) start))
      (assert (equal result expected) "replacement output differs")
      (when (= workload :unchanged) (assert (= result old)))
      (when (= workload :tail)
        (for [i 1 (- count 1)] (assert (= (. result.items i) (. old.items i)))))
      elapsed)
    (for [_ 1 6] (run baseline patches.baseline) (run candidate patches.candidate))
    (local samples {:baseline [] :candidate []})
    (for [iteration 1 10]
      (each [_ mode (ipairs (if (= (% iteration 2) 0) [:candidate :baseline] [:baseline :candidate]))]
        (local elapsed (run (if (= mode :baseline) baseline candidate) (. patches mode)))
        (table.insert (. samples mode) elapsed)))
    (each [mode times (pairs samples)]
      (table.sort times)
      ;; Separate allocation observation; do not mix GC-disabled timings into
      ;; the normal-GC CPU samples above.
      (collectgarbage :collect)
      (collectgarbage :stop)
      (local before (collectgarbage :count))
      (run (if (= mode :baseline) baseline candidate) (. patches mode))
      (local allocated (- (collectgarbage :count) before))
      (collectgarbage :restart)
      (print (string.format "%d,%s,%s,64_replacements,median_s=%.9f,best_s=%.9f,allocated_kib=%.3f"
                            count workload mode (/ (+ (. times 5) (. times 6)) 2) (. times 1) allocated)))))
