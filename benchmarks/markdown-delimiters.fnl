;; Isolate terminator searches and quote-prefix scanning. The quote depth changes
;; intentionally; both variants must preserve its body, and the new one its depth.
(local fennel (require :fennel))
(fn load-parser [path]
  (. (fennel.dofile path) :parse))

(fn equal [a b]
  (if (not= (type a) (type b))
      false
      (not= (type a) :table)
      (= a b)
      (and (accumulate [ok true k v (pairs a) &until (not ok)]
             (equal v (. b k)))
           (accumulate [ok true k (pairs b) &until (not ok)] (not= (. a k) nil)))))

(local parsers {:baseline (load-parser (assert (. arg 1)))
                :candidate (load-parser (assert (. arg 2)))})

(local samples (or (tonumber (. arg 3)) 10))
(each [_ spec (ipairs [{:name :brackets :unit "["}
                       {:name :links :unit "[x]("}
                       {:name :quotes :unit "> "}])]
  (each [_ size (ipairs [2000 4000 8000])]
    (local source (.. (string.rep spec.unit size) :tail))
    (local oracle {})
    (each [name parse (pairs parsers)]
      (tset oracle name (parse source))
      (for [_ 1 6]
        (assert (equal (parse source) (. oracle name))
                "nondeterministic parser")))
    (if (= spec.name :quotes)
        (do
          (assert (= (. oracle.candidate.blocks 1 :depth) size))
          (assert (= (. oracle.candidate.blocks 1 :inlines 1 :text) :tail))
          (assert (equal (. oracle.baseline.blocks 1 :inlines)
                         (. oracle.candidate.blocks 1 :inlines))))
        (assert (equal oracle.baseline oracle.candidate) "AST mismatch"))
    (local times {:baseline [] :candidate []})
    (for [sample 1 samples]
      (each [_ name (ipairs (if (= (% sample 2) 0) [:candidate :baseline]
                                [:baseline :candidate]))]
        (collectgarbage :collect)
        (local start (os.clock))
        (local document ((. parsers name) source))
        (local elapsed (- (os.clock) start))
        (assert (equal document (. oracle name)) "timed AST mismatch")
        (table.insert (. times name) elapsed)
        (print (string.format "sample,%s,%d,%s,%d,%.9f" spec.name
                              (length source) name sample elapsed))))
    (each [_ name (ipairs [:baseline :candidate])]
      (local durations (. times name))
      (table.sort durations)
      (print (string.format "summary,%s,%d,%s,n=%d,median_s=%.9f,best_s=%.9f"
                            spec.name (length source) name samples
                            (/ (+ (. durations (math.floor (/ (+ samples 1) 2)))
                                  (. durations (math.ceil (/ (+ samples 1) 2))))
                               2) (. durations 1))))))
