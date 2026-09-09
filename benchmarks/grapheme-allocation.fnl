;; Allocation observation, not native frame timing. GC is stopped only here.
;; tools/fennel benchmarks/grapheme-allocation.fnl /path/to/baseline/layout.fnl
(local fennel (require :fennel))
(require :tests.application)
(fn api [path] (. ((fennel.dofile path) {}) :services :layout))
(local baseline (api (assert (. arg 1) "baseline layout source required")))
(local candidate (api :extensions/misa/ui/layout.fnl))
(local cases [{:id :ascii :text (string.rep "ASCII " 64)}
              {:id :unicode :text (string.rep "é界👩‍💻🇺🇸क्ष" 64)}
              {:id :mixed :text (string.rep "text é 界 👩‍💻 " 64)}])
(each [_ sample (ipairs cases)]
  (local expected (baseline.width sample.text))
  (fn run [layout]
    (var total 0)
    (for [_ 1 64] (set total (+ total (layout.width sample.text))))
    (assert (= total (* expected 64)) "grapheme width changed"))
  (for [_ 1 6] (run baseline) (run candidate))
  (for [iteration 1 10]
    (each [_ mode (ipairs (if (= (% iteration 2) 0) [:candidate :baseline] [:baseline :candidate]))]
      (collectgarbage :collect)
      (collectgarbage :stop)
      (local before (collectgarbage :count))
      (run (if (= mode :baseline) baseline candidate))
      (local allocated (- (collectgarbage :count) before))
      (collectgarbage :restart)
      (print (string.format "%s,%s,%d,64_width_calls,allocated_kib=%.3f"
                            sample.id mode iteration allocated)))))
