;; Exhaustive codepoint widths and generated mixed-text layout against a saved API.
;; tools/fennel benchmarks/layout-parity.fnl /path/to/baseline/layout.fnl
(local fennel (require :fennel))
(local G (require :tests.generators))
(fn api [path] (. ((. (fennel.dofile path) :setup)) :fx 1 :value))
(local baseline (api (assert (. arg 1) "saved layout source required")))
(local candidate (api :extensions/layout.fnl))
(for [cp 0 1114111]
  (assert (= (baseline.cell_width cp) (candidate.cell_width cp))
          (.. "codepoint width changed: " cp)))
(local pieces ["" "ASCII " " " "é" "界" "👩‍💻" "🇺🇸" "क्ष" "क्x" "1️⃣"
               "\n" "\r\n" "\t" "\0" "́" "‍" "🏽" (string.char 255) (string.char 226 130)])
(fn results [layout text columns]
  (local (head last cells) (layout.clip text columns))
  (local (taken rest taken-cells) (layout.take text columns))
  {:clip [head last cells] :take [taken rest taken-cells] :width (layout.width text)
   :boundary (layout.boundary_at_or_before text columns)
   :previous (layout.previous_boundary text columns) :next (layout.next_boundary text columns)
   :wrapped (layout.wrap_spans [{:spans [{:text text :style :accent :link "https://example.test"}]}] columns)})
(local failure
       (G.for_all (G.vector (G.elements pieces))
                  (fn [fragments]
                    (local text (table.concat fragments))
                    (each [_ columns (ipairs [0 1 2 5 16 80])]
                      (assert (= (fennel.view (results baseline text columns))
                                 (fennel.view (results candidate text columns)))
                              "layout output changed")))
                  {:cases 500 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(print "layout parity passed: every codepoint width; 500 generated mixed-text cases")
