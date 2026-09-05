;; Pure terminal-cell layout primitives. The width tables intentionally mirror

;; src/terminal/width.zig so Lua projections and the native presenter agree.

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

(fn contains [intervals cp]
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

;; UAX #29 GB9c linkers, kept in lockstep with terminal/width.zig. This is

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

(fn indic-letter [cp]
  (or (or (or (or (and (>= cp 2304) (<= cp 3583))
                  (and (>= cp 4096) (<= cp 4255)))
              (and (>= cp 6016) (<= cp 6143)))
          (and (>= cp 43008) (<= cp 44031)))
      (and (>= cp 69632) (<= cp 73215))))

(fn cell-width [cp]
  (if (or (contains zero cp) (. virama cp)) 0
      (contains wide cp) 2
      1))

(fn regional [cp] (and (>= cp 127462) (<= cp 127487)))

(fn cluster [text at]
  (let [(cp size) (decode text at)]
    (if (not cp)
        (values at 0)
        (do
          (var next-at (+ at size))
          (var cells (cell-width cp))
          (var emoji false)
          (var after-virama false)
          (var done false)
          (local flag (regional cp))
          (while (and (not done) (<= next-at (length text)))
            (let [(following following-size) (decode text next-at)]
              (if (and after-virama (indic-letter following)
                       (not (contains zero following)))
                  (set (cells after-virama next-at)
                       (values (math.max cells (cell-width following)) false
                               (+ next-at following-size)))
                  (= following 8205)
                  (let [(joined joined-size) (decode text (+ next-at following-size))]
                    (if joined
                        (set (emoji next-at cells)
                             (values true (+ next-at following-size joined-size)
                                     (math.max cells (cell-width joined))))
                        (do
                          (set next-at (+ next-at following-size))
                          (set done true))))
                  (or (. virama following) (contains zero following))
                  (do
                    (when (or (= following 65039) (= following 8419)
                              (<= 127995 following 127999))
                      (set emoji true))
                    (when (. virama following) (set after-virama true))
                    (set next-at (+ next-at following-size)))
                  (and flag (regional following))
                  (do
                    (set (emoji next-at) (values true (+ next-at following-size)))
                    (set done true))
                  (set done true))))
          (values next-at (if emoji (math.max cells 2) cells))))))

(fn boundary-at-or-before [text cursor]
  (let [target (math.max 0 (math.min (length text)
                                   (math.floor (or (tonumber cursor) 0))))]
    (var at 1)
    (var previous 0)
    (var done false)
    (while (and (not done) (<= at (length text)))
      (let [next-at (cluster text at)
            boundary (- next-at 1)]
        (if (> boundary target)
            (set done true)
            (do
              (set (previous at) (values boundary next-at))
              (set done (= boundary target))))))
    previous))

(fn previous-boundary [text cursor]
  (let [target (boundary-at-or-before text cursor)]
    (var at 1)
    (var previous 0)
    (var done (= target 0))
    (while (and (not done) (<= at (length text)))
      (let [next-at (cluster text at)
            boundary (- next-at 1)]
        (if (>= boundary target)
            (set done true)
            (set (previous at) (values boundary next-at)))))
    previous))

(fn next-boundary [text cursor]
  (let [boundary (boundary-at-or-before text cursor)]
    (if (>= boundary (length text))
        (length text)
        (- (cluster text (+ boundary 1)) 1))))

(fn width [text]
  (var (at cells) (values 1 0))
  (while (<= at (length text))
    (local (next-at cluster-cells) (cluster text at))
    (set (cells at) (values (+ cells cluster-cells) next-at)))
  cells)

(fn take [text columns]
  (let [limit (math.max 0 (math.floor (or (tonumber columns) 0)))]
    (var at 1)
    (var cells 0)
    (var done false)
    (while (and (not done) (<= at (length text)))
      (let [(next-at cluster-cells) (cluster text at)
            cells-next (+ cells cluster-cells)]
        (if (> cells-next limit)
            (do
              ;; Wrapping must consume an oversized first grapheme to advance.
              (when (= cells 0) (set (at cells) (values next-at cells-next)))
              (set done true))
            (set (at cells) (values next-at cells-next)))))
    (values (text:sub 1 (- at 1)) (text:sub at) cells)))

;; Strict clipping differs from wrapping's take(): a too-wide first grapheme is

;; omitted rather than overflowing. Segmentation is over the complete semantic

;; line, so callers can project the returned byte boundary through style spans.

(fn clip [text columns]
  (let [limit (math.max 0 (math.floor (or (tonumber columns) 0)))]
    (var at 1)
    (var cells 0)
    (var done false)
    (while (and (not done) (<= at (length text)))
      (let [(next-at cluster-cells) (cluster text at)]
        (if (> (+ cells cluster-cells) limit)
            (set done true)
            (set (cells at) (values (+ cells cluster-cells) next-at)))))
    (values (text:sub 1 (- at 1)) (- at 1) cells)))

(fn fit [text columns]
  (let [limit (math.max 0 (math.floor (or (tonumber columns) 0)))]
    (if (= limit 0)
        ""
        (let [(head _ cells) (take (tostring (or text "")) limit)]
          (.. head (string.rep " " (math.max 0 (- limit cells))))))))

;; Span boundaries describe appearance, never word/grapheme boundaries. Wrap a

;; semantic line once, then project its byte ranges back through arbitrary spans.

(fn copy-span [source text first last]
  (let [result {}]
    (each [key value (pairs source)] (tset result key value))
    (set result.text text)
    (when (= (type source.source_start) :number)
      (set result.source_start (- (+ source.source_start (or first 1)) 1))
      (set result.source_end (+ source.source_start (or last (length text)))))
    result))

(fn clone-spans [spans]
  (let [result {}]
    (each [_ source (ipairs (or spans {}))]
      (tset result (+ (length result) 1) (copy-span source (or source.text ""))))
    result))

(fn wrap-ranges [text first-columns rest-columns trim words]
  (let [result {}]
    (var (start at used) (values 1 1 0))
    (var room (math.max 1 first-columns))
    (set-forcibly! rest-columns (math.max 1 (or rest-columns room)))
    (var (break-last break-next whitespace-start) nil)

    (fn emit [last next-at]
      (tset result (+ (length result) 1) {:first start : last})
      (set (start at used room) (values next-at next-at 0 rest-columns))
      (set (break-last break-next whitespace-start) (values nil nil nil)))

    (while (<= at (length text))
      (local (next-at cells) (cluster text at))
      (local byte (text:byte at))
      (local whitespace (or (= byte 32) (= byte 9)))
      (if (and (not= words false) whitespace)
          (do
            (set whitespace-start (or whitespace-start at))
            (if (and trim (> whitespace-start start))
                (set (break-last break-next)
                     (values (- whitespace-start 1) next-at))
                (<= (+ used cells) room)
                (set (break-last break-next) (values (- next-at 1) next-at))))
          (not whitespace) (set whitespace-start nil))
      (if (and (> used 0) (> (+ used cells) room))
          (if (and break-next (> break-next start))
              (do
                (var next-start break-next)
                (when trim
                  (while (or (= (text:byte next-start) 32)
                             (= (text:byte next-start) 9))
                    (set next-start (+ next-start 1))))
                (emit break-last next-start)) (emit (- at 1) at))
          (set (used at) (values (+ used cells) next-at))))
    (when (or (<= start (length text)) (= (length result) 0))
      (tset result (+ (length result) 1) {:first start :last (length text)}))
    result))

(fn flow-spans [spans columns first-prefix rest-prefix options]
  (set-forcibly! options (or options {}))
  (set-forcibly! columns (math.max 1 (math.floor (or (tonumber columns) 1))))
  (set-forcibly! first-prefix (or first-prefix {}))
  (set-forcibly! rest-prefix (or rest-prefix first-prefix))

  (fn prefix-room [prefix]
    (let [pieces {}]
      (each [_ item (ipairs prefix)]
        (tset pieces (+ (length pieces) 1) (or item.text "")))
      (math.max 1 (- columns (width (table.concat pieces))))))

  (local pieces {})
  (each [_ source (ipairs (or spans {}))]
    (tset pieces (+ (length pieces) 1) (or source.text "")))
  (local text (table.concat pieces))
  (var (result span-index span-at) (values {} 1 1))
  (local (first-room rest-room)
         (values (prefix-room first-prefix) (prefix-room rest-prefix)))
  (var line-start 1)
  (while line-start
    (local newline (text:find "\n" line-start true))
    (local line-end (or (and newline (- newline 1)) (length text)))
    (local ranges
           (wrap-ranges (text:sub line-start line-end)
                        (or (and (= (length result) 0) first-room) rest-room)
                        rest-room (not= options.trim false) options.words))
    (each [_ range (ipairs ranges)]
      (local (first last)
             (values (- (+ line-start range.first) 1)
                     (- (+ line-start range.last) 1)))
      (local current (clone-spans (or (and (= (length result) 0) first-prefix)
                                      rest-prefix)))
      (while (and (. spans span-index)
                  (< (- (+ span-at (length (or (. spans span-index :text) "")))
                        1) first))
        (set span-at (+ span-at (length (or (. spans span-index :text) ""))))
        (set span-index (+ span-index 1)))
      (var (index offset) (values span-index span-at))
      (while (and (. spans index) (<= offset last))
        (local source (. spans index))
        (local value (or source.text ""))
        (local (a b)
               (values (math.max 1 (+ (- first offset) 1))
                       (math.min (length value) (+ (- last offset) 1))))
        (when (>= b a)
          (tset current (+ (length current) 1)
                (copy-span source (value:sub a b) a b)))
        (set (offset index) (values (+ offset (length value)) (+ index 1))))
      (when (= (length current) 0)
        (tset current 1 (copy-span (or (. spans 1) {}) "")))
      (tset result (+ (length result) 1)
            {:source_end last :source_start (- first 1) :spans current}))
    (set line-start (and newline (+ newline 1))))
  result)

(fn wrap-spans [lines columns prefix options]
  (let [result {}]
    (each [_ line (ipairs (or lines {}))]
      (each [_ wrapped (ipairs (flow-spans (or line.spans {}) columns prefix
                                           prefix options))]
        ;; Preserve line decorations (surfaces, actions, etc.) without making the
        ;; layout service aware of feature-specific metadata.
        (each [key value (pairs line)]
          (when (not= key :spans) (tset wrapped key value)))
        (tset result (+ (length result) 1) wrapped)))
    result))

;; Wrap editable text into physical rows and project its global UTF-8 byte

;; cursor onto the resulting row. Prompt bytes are part of each semantic row,

;; so the native presenter can continue to own byte-to-cell conversion.

(fn normalize-newlines [text cursor]
  (set-forcibly! text (tostring (or text "")))
  (set-forcibly! cursor
                 (math.max 0
                           (math.min (length text)
                                     (math.floor (or (tonumber cursor) 0)))))
  (var (pieces mapped at bytes) (values {} nil 1 0))
  (while (<= at (length text))
    (when (and (= mapped nil) (>= (- at 1) cursor))
      (set mapped bytes))
    (if (= (text:byte at) 13)
        (let [size (or (and (= (text:byte (+ at 1)) 10) 2) 1)]
          (tset pieces (+ (length pieces) 1) "\n")
          (set bytes (+ bytes 1))
          (set at (+ at size))
          (when (and (= mapped nil) (>= (- at 1) cursor))
            (set mapped bytes)))
        (let [(_ size) (decode text at)]
          (tset pieces (+ (length pieces) 1) (text:sub at (- (+ at size) 1)))
          (set bytes (+ bytes size))
          (set at (+ at size)))))
  (values (table.concat pieces) (or mapped bytes)))

(fn wrap-input [text columns cursor prompt text-style prompt-style]
  (set-forcibly! (text cursor) (normalize-newlines text cursor))
  (set-forcibly! columns (math.max 1 (math.floor (or (tonumber columns) 1))))
  (set-forcibly! cursor (boundary-at-or-before text cursor))
  ;; Preserve two cells for a potentially-wide grapheme whenever possible;
  ;; on tiny terminals the prompt yields before editable content does.
  (set-forcibly! prompt (tostring (or prompt "")))
  (local prompt-room (math.max 0 (- columns 2)))
  (set-forcibly! prompt (or (and (= prompt-room 0) "")
                            (take prompt prompt-room)))
  (local prompt-cells (width prompt))
  (local room (math.max 1 (- columns prompt-cells)))
  (local continuation (string.rep " " prompt-cells))
  (var (rows mapped) (values {} nil))
  (var line-start 0)
  (while line-start
    (local newline (text:find "\n" (+ line-start 1) true))
    (local line-end (or (and newline (- newline 1)) (length text)))
    (local value (text:sub (+ line-start 1) line-end))
    ;; Keep all source whitespace in the editor so selection/cursor offsets are
    ;; exact; prose rendering can discard separators at a soft wrap instead.
    (local segments (wrap-ranges value room room false))
    (each [index segment (ipairs segments)]
      (local prefix (or (and (= (length rows) 0) prompt) continuation))
      (local piece (value:sub segment.first segment.last))
      (tset rows (+ (length rows) 1)
            {:spans [{:style prompt-style :text prefix}
                     {:style text-style :text piece}]})
      (local first-byte (- (+ line-start segment.first) 1))
      (local last-byte (+ line-start segment.last))
      ;; Prefer the following physical row at a soft-wrap boundary. This keeps
      ;; an insertion cursor off the unusable cell just beyond the right edge.
      (when (and (>= cursor first-byte) (<= cursor last-byte))
        (set mapped {:byte (+ (length prefix) (- cursor first-byte))
                     :row (length rows)})))
    (set line-start newline))
  {:cursor (or mapped {:byte (length (.. (or (. rows (length rows) :spans 1
                                                :text)
                                             "")
                                         (or (. rows (length rows) :spans 2
                                                :text)
                                             "")))
                       :row (length rows)})
   :lines rows})

(fn columns [total minimum maximum gap]
  (set-forcibly! (total minimum maximum gap)
                 (values (math.max 1 (math.floor total))
                         (math.max 1 (math.floor minimum))
                         (math.max 1 (math.floor maximum))
                         (math.max 0 (math.floor (or gap 0)))))
  (local count
         (math.max 1
                   (math.min maximum
                             (math.floor (/ (+ total gap) (+ minimum gap))))))
  (local (usable widths) (values (math.max count (- total (* gap (- count 1))))
                                 {}))
  (local (base extra) (values (math.floor (/ usable count)) (% usable count)))
  (for [index 1 count]
    (tset widths index (+ base (or (and (<= index extra) 1) 0))))
  widths)

(local api {:boundary_at_or_before boundary-at-or-before
            :cell_width cell-width
            : clip
            : columns
            : fit
            :flow_spans flow-spans
            :next_boundary next-boundary
            :previous_boundary previous-boundary
            : take
            : width
            :wrap_input wrap-input
            :wrap_ranges wrap-ranges
            :wrap_spans wrap-spans})

{:setup (fn [] (set misa.layout api))}

