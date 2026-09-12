;; Pure terminal-cell layout primitives. Measurement and grapheme segmentation
;; come from the native module in `src/width/root.zig`, which the runtime installs
;; as `misa.native`, so Lua projections and the native presenter cannot disagree
;; about cell geometry.
;;
;; The standalone Fennel harness has no native module: `tools/fennel` registers
;; the offline stand-in in `tests/native-layout.fnl` under the same name, and the
;; layout-parity integration case compares that stand-in with the native module.
;; Without either, this module fails to load instead of measuring text differently.

(local native (require :misa.native))

(fn byte-clusters [text]
  "Cluster boundaries for input the native measurement rejects."
  (let [result {}]
    (for [at 1 (length text)]
      (tset result (+ (length result) 1) at)
      (tset result (+ (length result) 1) 1))
    result))

;; The native measurement rejects malformed UTF-8. The previous Lua
;; implementation measured such input one byte at a time, so the fallback keeps
;; that behavior: every byte is its own boundary and occupies one cell.

(fn clusters [text]
  "Cluster boundaries as last byte, cells, last byte, cells, ..."
  (or (and (= (type text) :string) (native.clusters text)) (byte-clusters text)))

(fn cells-in [clusters]
  (var (index cells) (values 1 0))
  (while (<= index (length clusters))
    (set (cells index) (values (+ cells (. clusters (+ index 1))) (+ index 2))))
  cells)

(fn width [text]
  "Measure text in terminal cells."
  (let [measured (and (= (type text) :string) (native.width text))]
    (if measured measured (cells-in (clusters text)))))

(fn cell-width [cp]
  "Return the terminal cell width of a Unicode codepoint."
  (native.cell-width cp))

(fn boundary-at-or-before [text cursor]
  "Clamp a byte cursor to the preceding grapheme boundary."
  (let [target (math.max 0
                         (math.min (length text)
                                   (math.floor (or (tonumber cursor) 0))))
        clusters (clusters text)]
    (var boundary 0)
    (for [index 1 (length clusters) 2 &until (> (. clusters index) target)]
      (set boundary (. clusters index)))
    boundary))

(fn previous-boundary [text cursor]
  "Return the grapheme boundary before a byte cursor."
  (let [target (boundary-at-or-before text cursor)
        clusters (clusters text)]
    (var previous 0)
    (for [index 1 (length clusters) 2 &until (>= (. clusters index) target)]
      (set previous (. clusters index)))
    previous))

(fn next-boundary [text cursor]
  "Return the grapheme boundary after a byte cursor."
  (let [boundary (boundary-at-or-before text cursor)]
    (if (>= boundary (length text))
        (length text)
        (let [target (+ boundary 1)
              clusters (clusters text)]
          (var (index next) (values 1 nil))
          (while (and (not next) (<= index (length clusters)))
            (let [last (. clusters index)]
              (if (>= last target)
                  (set next last)
                  (set index (+ index 2)))))
          ;; The final cluster ends at the end of the text, so this is bounded.
          (or next (length text))))))

(fn take [text columns]
  "Take the longest grapheme-aligned prefix that fits a cell width."
  (let [limit (math.max 0 (math.floor (or (tonumber columns) 0)))
        clusters (clusters text)]
    (var (index at cells) (values 1 1 0))
    (var done false)
    (while (and (not done) (<= index (length clusters)))
      (let [last (. clusters index)
            cluster-cells (. clusters (+ index 1))
            cells-next (+ cells cluster-cells)]
        (if (> cells-next limit)
            (do
              ;; Wrapping must consume an oversized first grapheme to advance.
              (when (= cells 0)
                (set (at cells) (values (+ last 1) cells-next)))
              (set done true))
            (set (at cells index) (values (+ last 1) cells-next (+ index 2))))))
    (values (text:sub 1 (- at 1)) (text:sub at) cells)))

;; Strict clipping differs from wrapping's take(): a too-wide first grapheme is
;; omitted rather than overflowing. Segmentation is over the complete semantic
;; line, so callers can project the returned byte boundary through style spans.

(fn clip [text columns]
  "Clip text to a terminal cell width with an ellipsis when needed."
  (let [limit (math.max 0 (math.floor (or (tonumber columns) 0)))
        clusters (clusters text)]
    (var (index at cells) (values 1 1 0))
    (var done false)
    (while (and (not done) (<= index (length clusters)))
      (let [last (. clusters index)
            cluster-cells (. clusters (+ index 1))]
        (if (> (+ cells cluster-cells) limit)
            (set done true)
            (set (at cells index)
                 (values (+ last 1) (+ cells cluster-cells) (+ index 2))))))
    (values (text:sub 1 (- at 1)) (- at 1) cells)))

(fn fit [text columns]
  "Clip or pad text to exactly the requested cell width."
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
    ;; Clock frames preserve cell geometry only when the whole span survives.
    (when (not= text source.text) (set result.animation nil))
    (when (and (= (type source.source_start) :number)
               (or (= source.source_end nil)
                   (= (- source.source_end source.source_start)
                      (length (or source.text "")))))
      (set result.source_start (- (+ source.source_start (or first 1)) 1))
      (set result.source_end (+ source.source_start (or last (length text)))))
    result))

(fn clone-spans [spans]
  (let [result {}]
    (each [_ source (ipairs (or spans {}))]
      (tset result (+ (length result) 1) (copy-span source (or source.text ""))))
    result))

(fn wrap-ranges [text first-columns rest-columns trim words]
  "Split text into byte ranges that fit successive row widths."
  (let [result {}
        clusters (clusters text)]
    (var (start at used index) (values 1 1 0 1))
    (var room (math.max 1 first-columns))
    (let [rest-columns (math.max 1 (or rest-columns room))]
      ;; `index` always names the cluster that starts at `at`, so a break records
      ;; the cluster it resumes at alongside the byte it resumes from.
      (var (break-last break-next break-index whitespace-start) nil)

      (fn emit [last next-at next-index]
        (tset result (+ (length result) 1) {:first start : last})
        (set (start at used index room)
             (values next-at next-at 0 next-index rest-columns))
        (set (break-last break-next break-index whitespace-start)
             (values nil nil nil nil)))

      (while (<= index (length clusters))
        (let [last (. clusters index)
              cells (. clusters (+ index 1))
              next-at (+ last 1)
              byte (text:byte at)
              whitespace (or (= byte 32) (= byte 9))]
          (if (and (not= words false) whitespace)
              (do
                (set whitespace-start (or whitespace-start at))
                (if (and trim (> whitespace-start start))
                    (set (break-last break-next break-index)
                         (values (- whitespace-start 1) next-at (+ index 2)))
                    (<= (+ used cells) room)
                    (set (break-last break-next break-index)
                         (values (- next-at 1) next-at (+ index 2)))))
              (not whitespace)
              (set whitespace-start nil))
          (if (and (> used 0) (> (+ used cells) room))
              (if (and break-next (> break-next start))
                  (do
                    (var next-start break-next)
                    (var next-index break-index)
                    (when trim
                      (while (or (= (text:byte next-start) 32)
                                 (= (text:byte next-start) 9))
                        (set next-start (+ next-start 1))
                        (set next-index (+ next-index 2))))
                    (emit break-last next-start next-index))
                  (emit (- at 1) at index))
              (set (used at index) (values (+ used cells) next-at (+ index 2))))))
      (when (or (<= start (length text)) (= (length result) 0))
        (tset result (+ (length result) 1) {:first start :last (length text)}))
      result)))

(fn flow-spans [spans columns first-prefix rest-prefix options]
  "Wrap styled spans while preserving source coordinates and row prefixes."
  (let [options (or options {})
        columns (math.max 1 (math.floor (or (tonumber columns) 1)))
        first-prefix (or first-prefix {})
        rest-prefix (or rest-prefix first-prefix)]
    (fn prefix-room [prefix]
      (let [pieces {}]
        (each [_ item (ipairs prefix)]
          (tset pieces (+ (length pieces) 1) (or item.text "")))
        (math.max 1 (- columns (width (table.concat pieces))))))

    (let [pieces {}]
      (each [_ source (ipairs (or spans {}))]
        (tset pieces (+ (length pieces) 1) (or source.text "")))
      (let [text (table.concat pieces)]
        (var (result span-index span-at) (values {} 1 1))
        (let [(first-room rest-room) (values (prefix-room first-prefix)
                                             (prefix-room rest-prefix))]
          (var line-start 1)
          (while line-start
            (let [newline (text:find "\n" line-start true)
                  line-end (or (and newline (- newline 1)) (length text))
                  ranges (wrap-ranges (text:sub line-start line-end)
                                      (or (and (= (length result) 0) first-room)
                                          rest-room)
                                      rest-room (not= options.trim false)
                                      options.words)]
              (each [_ range (ipairs ranges)]
                (let [(first last) (values (- (+ line-start range.first) 1)
                                           (- (+ line-start range.last) 1))
                      current (clone-spans (or (and (= (length result) 0)
                                                    first-prefix)
                                               rest-prefix))]
                  (while (and (. spans span-index)
                              (< (- (+ span-at
                                       (length (or (. spans span-index :text)
                                                   "")))
                                    1) first))
                    (set span-at
                         (+ span-at (length (or (. spans span-index :text) ""))))
                    (set span-index (+ span-index 1)))
                  (var (index offset) (values span-index span-at))
                  (while (and (. spans index) (<= offset last))
                    (let [source (. spans index)
                          value (or source.text "")
                          (a b) (values (math.max 1 (+ (- first offset) 1))
                                        (math.min (length value)
                                                  (+ (- last offset) 1)))]
                      (when (>= b a)
                        (tset current (+ (length current) 1)
                              (copy-span source (value:sub a b) a b)))
                      (set (offset index)
                           (values (+ offset (length value)) (+ index 1)))))
                  (when (= (length current) 0)
                    (tset current 1 (copy-span (or (. spans 1) {}) "")))
                  (tset result (+ (length result) 1)
                        {:source_end last
                         :source_start (- first 1)
                         :spans current})))
              (set line-start (and newline (+ newline 1)))))
          result)))))

(fn wrap-spans [lines columns prefix options]
  "Wrap lines of styled spans to a terminal cell width."
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

(fn utf8-size [text at]
  "Byte length of the character at `at`; a malformed sequence advances one byte."
  (let [first (text:byte at)]
    (if (not first)
        0
        (let [size (if (<= 194 first 223) 2
                       (<= 224 first 239) 3
                       (<= 240 first 244) 4
                       1)]
          (var valid true)
          (for [index (+ at 1) (- (+ at size) 1) &until (not valid)]
            (let [byte (text:byte index)]
              (when (or (not byte) (< byte 128) (>= byte 192))
                (set valid false))))
          (if valid size 1)))))

(fn normalize-newlines [text cursor]
  (let [text (tostring (or text ""))
        cursor (math.max 0
                         (math.min (length text)
                                   (math.floor (or (tonumber cursor) 0))))]
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
          (let [size (math.max 1 (utf8-size text at))]
            (tset pieces (+ (length pieces) 1) (text:sub at (- (+ at size) 1)))
            (set bytes (+ bytes size))
            (set at (+ at size)))))
    (values (table.concat pieces) (or mapped bytes))))

(fn wrap-input [text columns cursor prompt text-style prompt-style]
  "Lay out editable text and map its byte cursor to a terminal row."
  (let [(text cursor) (normalize-newlines text cursor)
        columns (math.max 1 (math.floor (or (tonumber columns) 1)))
        cursor (boundary-at-or-before text cursor)
        ;; Preserve two cells for a potentially-wide grapheme whenever possible;
        ;; on tiny terminals the prompt yields before editable content does.
        prompt (tostring (or prompt ""))
        prompt-room (math.max 0 (- columns 2))]
    (let [prompt (or (and (= prompt-room 0) "") (take prompt prompt-room))
          prompt-cells (width prompt)
          room (math.max 1 (- columns prompt-cells))
          continuation (string.rep " " prompt-cells)]
      (var (rows mapped) (values {} nil))
      (var line-start 0)
      (while line-start
        (let [newline (text:find "\n" (+ line-start 1) true)
              line-end (or (and newline (- newline 1)) (length text))
              value (text:sub (+ line-start 1) line-end)
              ;; Keep all source whitespace in the editor so selection/cursor offsets are
              ;; exact; prose rendering can discard separators at a soft wrap instead.
              segments (wrap-ranges value room room false)]
          (each [index segment (ipairs segments)]
            (let [prefix (or (and (= (length rows) 0) prompt) continuation)
                  piece (value:sub segment.first segment.last)]
              (tset rows (+ (length rows) 1)
                    {:spans [{:style prompt-style :text prefix}
                             {:style text-style :text piece}]})
              (let [first-byte (- (+ line-start segment.first) 1)
                    last-byte (+ line-start segment.last)]
                ;; Prefer the following physical row at a soft-wrap boundary. This keeps
                ;; an insertion cursor off the unusable cell just beyond the right edge.
                (when (and (>= cursor first-byte) (<= cursor last-byte))
                  (set mapped
                       {:byte (+ (length prefix) (- cursor first-byte))
                        :row (length rows)})))))
          (set line-start newline)))
      {:cursor (or mapped {:byte (length (.. (or (. rows (length rows) :spans 1
                                                    :text)
                                                 "")
                                             (or (. rows (length rows) :spans 2
                                                    :text)
                                                 "")))
                           :row (length rows)})
       :lines rows})))

(fn columns [total minimum maximum gap]
  "Allocate column widths within the available terminal cells."
  (let [(total minimum maximum gap) (values (math.max 1 (math.floor total))
                                            (math.max 1 (math.floor minimum))
                                            (math.max 1 (math.floor maximum))
                                            (math.max 0 (math.floor (or gap 0))))
        count (math.max 1
                        (math.min maximum
                                  (math.floor (/ (+ total gap) (+ minimum gap)))))
        (usable widths) (values (math.max count (- total (* gap (- count 1))))
                                {})
        (base extra) (values (math.floor (/ usable count)) (% usable count))]
    (for [index 1 count]
      (tset widths index (+ base (or (and (<= index extra) 1) 0))))
    widths))

(local api {: boundary-at-or-before
            : cell-width
            : clip
            : columns
            : fit
            : flow-spans
            : next-boundary
            : previous-boundary
            : take
            : width
            : wrap-input
            : wrap-ranges
            : wrap-spans})

api
