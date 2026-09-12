;; Offline stand-in for the native terminal measurement.
;;
;; The presenter measures frames with `src/width/root.zig` and `misa.ui.layout`
;; measures text through the same module, which the shipped runtime installs as
;; `misa.native`. The standalone Fennel harness (`tools/fennel`) runs ordinary
;; LuaJIT with no native module, so it registers this file under that name
;; instead.
;;
;; This is a test double, not a second implementation of the shipped rules. The
;; layout-parity integration case compares it with `misa.native` over every
;; codepoint and the grapheme corpus, so a divergence between this file and the
;; native module fails the suite instead of silently changing what the standalone
;; tests measure.

(local zero [[768 879]
             [1155 1161]
             [1425 1469]
             [1471 1471]
             [1473 1474]
             [1476 1477]
             [1552 1562]
             [1611 1631]
             [1648 1648]
             [1750 1773]
             [1809 1809]
             [1840 1866]
             [1958 1968]
             [2027 2035]
             [2070 2093]
             [2137 2139]
             [2259 2306]
             [2362 2364]
             [2369 2376]
             [2381 2381]
             [2385 2391]
             [2402 2403]
             [6832 6911]
             [7616 7679]
             [8203 8207]
             [8234 8238]
             [8288 8303]
             [8400 8447]
             [65024 65039]
             [65056 65071]
             [65279 65279]
             [127995 127999]
             [917536 917631]
             [917760 917999]])

(local wide [[4352 4447]
             [8986 8987]
             [9001 9002]
             [9193 9196]
             [9200 9200]
             [9203 9203]
             [9725 9726]
             [9748 9749]
             [9800 9811]
             [9855 9855]
             [9875 9875]
             [9889 9889]
             [9898 9899]
             [9917 9918]
             [9924 9925]
             [9934 9934]
             [9940 9940]
             [9962 9962]
             [9970 9971]
             [9973 9973]
             [9978 9978]
             [9981 9981]
             [9989 9989]
             [9994 9995]
             [10024 10024]
             [10060 10060]
             [10062 10062]
             [10067 10069]
             [10071 10071]
             [10133 10135]
             [10160 10160]
             [10175 10175]
             [11035 11036]
             [11088 11088]
             [11093 11093]
             [11904 12350]
             [12352 42191]
             [44032 55203]
             [63744 64255]
             [65040 65049]
             [65072 65135]
             [65280 65376]
             [65504 65510]
             [126980 126980]
             [127183 127183]
             [127374 127374]
             [127377 127386]
             [127488 127569]
             [127744 128591]
             [128640 128767]
             [129280 129535]
             [129648 129791]
             [131072 262141]])

(fn contains? [intervals cp]
  (var low 1)
  (var high (length intervals))
  (var found false)
  (while (and (<= low high) (not found))
    (let [middle (math.floor (/ (+ low high) 2))
          range (. intervals middle)]
      (if (< cp (. range 1)) (set high (- middle 1))
          (> cp (. range 2)) (set low (+ middle 1))
          (set found true))))
  found)

(fn decode [text at]
  (let [a (text:byte at)]
    (if (not a)
        (values nil 0)
        (let [(size initial) (if (<= 194 a 223) (values 2 (- a 192))
                                 (<= 224 a 239) (values 3 (- a 224))
                                 (<= 240 a 244) (values 4 (- a 240))
                                 (values 1 a))]
          (var cp initial)
          (var valid true)
          (for [index (+ at 1) (- (+ at size) 1) &until (not valid)]
            (let [byte (text:byte index)]
              (if (or (not byte) (< byte 128) (>= byte 192))
                  (set valid false)
                  (set cp (- (+ (* cp 64) byte) 128)))))
          (if valid (values cp size) (values a 1))))))

;; UAX #29 GB9c linkers, kept in lockstep with width/root.zig. This is
;; deliberately conservative: only a linker followed by an Indic letter joins.

(local virama {})

(each [_ cp (ipairs [2381
                     2509
                     2637
                     2765
                     2893
                     3021
                     3149
                     3277
                     3387
                     3388
                     3405
                     3530
                     3642
                     3972
                     4153
                     4154
                     5908
                     5909
                     5940
                     6098
                     6752
                     6980
                     7082
                     7083
                     7154
                     7155
                     43014
                     43204
                     43347
                     43456
                     43766
                     44013
                     68159
                     69702
                     69744
                     69939
                     69940
                     70080
                     70197
                     70378
                     70477
                     70722
                     70850
                     71103
                     71104
                     71231
                     71350
                     71467
                     71737
                     71997
                     71998
                     72003
                     72160
                     72244
                     72263
                     72345
                     72767
                     73028
                     73029
                     73111])]
  (tset virama cp true))

(fn indic-letter? [cp]
  (or (and (>= cp 2304) (<= cp 3583)) (and (>= cp 4096) (<= cp 4255))
      (and (>= cp 6016) (<= cp 6143)) (and (>= cp 43008) (<= cp 44031))
      (and (>= cp 69632) (<= cp 73215))))

(fn cell-width [cp]
  "Return the terminal cell width of a Unicode codepoint, or nil off Unicode."
  (when (and (= (type cp) :number) (>= cp 0) (<= cp 1114111))
    (if (or (contains? zero cp) (. virama cp)) 0
        (contains? wide cp) 2
        1)))

(fn regional? [cp] (and (>= cp 127462) (<= cp 127487)))

(fn cluster [text at]
  "Return the last byte and cell width of the cluster starting at `at`."
  (let [(cp size) (decode text at)]
    (if (not cp)
        (values (- at 1) 0)
        (do
          (var next-at (+ at size))
          (var cells (cell-width cp))
          (var emoji false)
          (var after-virama false)
          (var done false)
          (let [flag (regional? cp)]
            (while (and (not done) (<= next-at (length text)))
              (let [(following following-size) (decode text next-at)]
                (if (and after-virama (indic-letter? following)
                         (not (contains? zero following)))
                    (set (cells after-virama next-at)
                         (values (math.max cells (cell-width following)) false
                                 (+ next-at following-size)))
                    (= following 8205)
                    (let [(joined joined-size) (decode text
                                                       (+ next-at
                                                          following-size))]
                      (if joined
                          (set (emoji next-at cells)
                               (values true
                                       (+ next-at following-size joined-size)
                                       (math.max cells (cell-width joined))))
                          (do
                            (set next-at (+ next-at following-size))
                            (set done true))))
                    (or (. virama following) (contains? zero following))
                    (do
                      (when (or (= following 65039) (= following 8419)
                                (<= 127995 following 127999))
                        (set emoji true))
                      (when (. virama following) (set after-virama true))
                      (set next-at (+ next-at following-size)))
                    (and flag (regional? following))
                    (do
                      (set (emoji next-at)
                           (values true (+ next-at following-size)))
                      (set done true))
                    (set done true))))
            ;; A conditional in the final values position compiles to a thunk in
            ;; Fennel. Bind the scalar first: segmentation must not allocate one
            ;; closure per grapheme merely to return its cell count.
            (let [cluster-cells (if emoji (math.max cells 2) cells)]
              (values (- next-at 1) cluster-cells)))))))

(fn width [text]
  "Measure text in terminal cells."
  (var (at cells) (values 1 0))
  (while (<= at (length text))
    (let [(last cluster-cells) (cluster text at)]
      (set (at cells) (values (+ last 1) (+ cells cluster-cells)))))
  cells)

(fn clusters [text]
  "Cluster boundaries as last byte, cells, last byte, cells, ..."
  (let [result {}]
    (var at 1)
    (while (<= at (length text))
      (let [(last cells) (cluster text at)]
        (tset result (+ (length result) 1) last)
        (tset result (+ (length result) 1) cells)
        (set at (+ last 1))))
    result))

{: cell-width
 : clusters
 : width}
