;; Semantic terminal flow for documents produced by the markdown extension.

;; It owns layout, but only emits theme tokens; theme resolution remains at the

;; component registry boundary.

(local syntax-classes {:attribute true
                       :comment true
                       :constant true
                       :embedded true
                       :escape true
                       :function true
                       :keyword true
                       :number true
                       :operator true
                       :property true
                       :punctuation true
                       :string true
                       :tag true
                       :type true
                       :variable true})

(local heading-marks [:markdown.heading.1
                      :markdown.heading.2
                      :markdown.heading.3
                      :markdown.heading.4
                      :markdown.heading.5
                      :markdown.heading.6])

(local heading-prefix ["█ " "▌ " "▸ " "▪ " "· " "· "])

(fn compose [base ...]
  (let [result {}]
    (if (= (type base) :table)
        (each [_ token (ipairs base)]
          (tset result (+ (length result) 1) token))
        (tset result 1 (or base :plain)))
    (for [index 1 (select "#" ...)]
      (local token (select index ...))
      (when token
        (tset result (+ (length result) 1) token)))
    result))

(fn style-key [base]
  (if (not= (type base) :table) (.. (type base) ":" (tostring base))
      (let [parts ["tokens:"]]
        (each [_ token (ipairs base)]
          (tset parts (+ (length parts) 1) (.. (length token) ":" token)))
        (table.concat parts))))

(fn span [text style link] {: link : style : text})

(fn inline-spans [nodes base extra]
  (let [result {}]
    (fn visit [items link]
      (each [_ node (ipairs (or items {}))]
        (if (= node.kind :link) (visit node.children node.target)
            (let [style (compose base extra)]
              (each [_ mark (ipairs (or node.marks {}))]
                (if (= mark :strong) (tset style (+ (length style) 1) :bold)
                    (= mark :emphasis) (tset style (+ (length style) 1) :italic)
                    (= mark :strong_emphasis)
                    (do
                      (tset style (+ (length style) 1) :bold)
                      (tset style (+ (length style) 1) :italic))
                    (= mark :strikethrough)
                    (tset style (+ (length style) 1) :strikethrough)))
              (when (= node.kind :code)
                (tset style (+ (length style) 1) :code))
              (when link
                (tset style (+ (length style) 1) :link))
              (tset result (+ (length result) 1)
                    (span (or node.text "") style link))
              (tset (. result (length result)) :source true))))
      nil)

    (visit nodes nil)
    (when (= (length result) 0)
      (tset result 1 (span "" (compose base extra))))
    result))

(fn flow [spans columns first-prefix rest-prefix options]
  (misa.layout.flow_spans spans columns first-prefix rest-prefix options))

(fn append [target lines]
  (each [_ line (ipairs lines)]
    (tset target (+ (length target) 1) line))
  nil)

(fn pad-spans [spans width alignment base]
  (var used 0)
  (each [_ item (ipairs spans)]
    (set used (+ used (misa.layout.width (or item.text "")))))
  (local gap (math.max 0 (- width used)))
  (local left
         (or (and (= alignment :right) gap)
             (or (and (= alignment :center) (math.floor (/ gap 2))) 0)))
  (local right (- gap left))
  (local result {})
  (when (> left 0)
    (tset result (+ (length result) 1) (span (string.rep " " left) base)))
  (each [_ item (ipairs spans)]
    (tset result (+ (length result) 1) item))
  (when (> right 0)
    (tset result (+ (length result) 1) (span (string.rep " " right) base)))
  result)

(fn cell-lines [nodes width base header]
  (flow (inline-spans nodes base (or (and header :markdown.table.header) nil))
        (math.max 1 width) {} {}))

(fn allocate-columns [table-block available]
  (let [count (length table-block.align)
        widths {}]
    (for [column 1 count] (tset widths column 3))
    (each [_ row (ipairs table-block.rows)]
      (for [column 1 count]
        (var natural 0)
        (each [_ item (ipairs (inline-spans (or (. row column) {}) :plain))]
          (set natural (+ natural (misa.layout.width (or item.text "")))))
        (tset widths column (math.max (. widths column) (math.min 40 natural)))))
    (local room
           (math.max (* 2 count) (- (- available (+ count 1)) (* 2 count))))
    (var total 0)
    (each [_ width (ipairs widths)] (set total (+ total width)))
    (while (> total room)
      (var widest 1)
      (for [column 2 count]
        (when (> (. widths column) (. widths widest)) (set widest column)))
      (when (<= (. widths widest) 2) (lua :break))
      (tset widths widest (- (. widths widest) 1))
      (set total (- total 1)))
    (while (< total room)
      (var grew false)
      (for [column 1 count]
        (when (and (< total room) (< (. widths column) 40))
          (tset widths column (+ (. widths column) 1))
          (set total (+ total 1))
          (set grew true)))
      (when (not grew) (lua :break)))
    widths))

(fn border [widths left middle right base]
  (let [pieces [left]]
    (each [index width (ipairs widths)]
      (tset pieces (+ (length pieces) 1) (string.rep "─" (+ width 2)))
      (tset pieces (+ (length pieces) 1)
            (or (and (= index (length widths)) right) middle)))
    {:spans [(span (table.concat pieces) (compose base :markdown.table.border))]}))

(fn render-table [block columns base]
  ;; A complete table needs borders, padding, and room for a wide grapheme
  ;; in every cell. Below that width, preserve its fields in a stacked view.
  (if (< columns (+ (* 5 (length block.align)) 1))
      (let [result {}
            header (or (. block.rows 1) {})]
        (for [row-index 2 (length block.rows)]
          (when (> row-index 2)
            (tset result (+ (length result) 1) {:spans [(span "" base)]}))
          (for [column 1 (length block.align)]
            (local spans
                   (inline-spans (or (. header column) {}) base
                                 :markdown.table.header))
            (tset spans (+ (length spans) 1) (span ": " base))
            (each [_ item (ipairs (inline-spans (or (. block.rows row-index
                                                       column)
                                                    {})
                                                base))]
              (tset spans (+ (length spans) 1) item))
            (append result (flow spans columns))))
        (when (= (length block.rows) 1)
          (each [_ cell (ipairs header)]
            (append result (flow (inline-spans cell base :markdown.table.header)
                                 columns))))
        result)
      (let [widths (allocate-columns block columns)
            result [(border widths "┌" "┬" "┐" base)]]
        (each [row-index row (ipairs block.rows)]
          (var (cells height) (values {} 1))
          (each [column width (ipairs widths)]
            (tset cells column
                  (cell-lines (or (. row column) {}) width base (= row-index 1)))
            (set height (math.max height (length (. cells column)))))
          (for [line-index 1 height]
            (local pieces [(span "│" (compose base :markdown.table.border))])
            (each [column width (ipairs widths)]
              (tset pieces (+ (length pieces) 1) (span " " base))
              (local content (. (or (. cells column line-index)
                                    {:spans [(span "" base)]})
                                :spans))
              (each [_ item (ipairs (pad-spans content width
                                               (. block.align column) base))]
                (tset pieces (+ (length pieces) 1) item))
              (tset pieces (+ (length pieces) 1)
                    (span " │" (compose base :markdown.table.border))))
            (tset result (+ (length result) 1) {:spans pieces}))
          (when (< row-index (length block.rows))
            (tset result (+ (length result) 1)
                  (border widths "├" "┼" "┤" base))))
        (tset result (+ (length result) 1)
              (border widths "└" "┴" "┘" base))
        result)))

(fn highlighted-lines [block base captures]
  (let [source (or block.text "")]
    (set-forcibly! captures (or captures {}))
    (var (spans at) (values {} 1))
    (each [_ capture (ipairs captures)]
      (local first (+ (or (tonumber capture.start_byte) 0) 1))
      (local after (+ (or (tonumber capture.end_byte) 0) 1))
      (local class capture.capture)
      (when (and (and (and (. syntax-classes class) (>= first at))
                      (> after first))
                 (<= after (+ (length source) 1)))
        (when (> first at)
          (tset spans (+ (length spans) 1)
                (span (source:sub at (- first 1)) (compose base :code))))
        (tset spans (+ (length spans) 1)
              (span (source:sub first (- after 1))
                    (compose base :code (.. :syntax. class))))
        (set at after)))
    (when (<= at (length source))
      (tset spans (+ (length spans) 1)
            (span (source:sub at) (compose base :code))))
    (when (= (length spans) 0)
      (tset spans 1 (span source (compose base :code))))
    (local lines [{:spans {}}])
    (each [_ item (ipairs spans)]
      (var start 1)
      (while true
        (local newline (item.text:find "\n" start true))
        (when (not newline)
          (tset (. lines (length lines) :spans)
                (+ (length (. lines (length lines) :spans)) 1)
                (span (item.text:sub start) item.style item.link))
          (lua :break))
        (tset (. lines (length lines) :spans)
              (+ (length (. lines (length lines) :spans)) 1)
              (span (item.text:sub start (- newline 1)) item.style item.link))
        (tset lines (+ (length lines) 1) {:spans {}})
        (set start (+ newline 1))))
    (each [_ line (ipairs lines)]
      (each [_ item (ipairs line.spans)] (set item.source true)))
    lines))

(fn code-block [block columns base captures]
  (let [language (or (and (not= block.language "") block.language) :plain)
        result {}
        label (misa.layout.take language (math.max 1 (- columns 4)))
        label-width (misa.layout.width label)]
    (tset result 1
          {:spans [(span (.. "┌─ " label " ")
                         (compose base :markdown.code.label))
                   (span (string.rep "─"
                                     (math.max 0 (- (- columns label-width) 4)))
                         (compose base :markdown.code.border))]})
    (each [_ line (ipairs (highlighted-lines block base captures))]
      (append result (flow line.spans columns
                           [(span "│ " (compose base :markdown.code.border))]
                           [(span "│ " (compose base :markdown.code.border))]
                           {:trim false :words false})))
    (tset result (+ (length result) 1)
          {:spans [(span (.. "└"
                             (string.rep "─" (math.max 0 (- columns 1))))
                         (compose base :markdown.code.border))]})
    result))

(fn render-block [block columns base captures]
  (let [result {}]
    (if (= block.kind :blank)
        (tset result (+ (length result) 1) {:spans [(span "" base)]})
        (= block.kind :paragraph)
        (append result (flow (inline-spans block.inlines base) columns))
        (= block.kind :heading)
        (let [token (or (. heading-marks block.level) (. heading-marks 6))
              prefix [(span (or (. heading-prefix block.level) "· ")
                            (compose base token))]]
          (append result
                  (flow (inline-spans block.inlines base token) columns prefix
                        [(span "  " base)])))
        (= block.kind :quote)
        (let [rails {}]
          ;; Preserve semantic depth while leaving room for a wide grapheme.
          (for [_ 1 (math.min (math.max 1 (or block.depth 1))
                              (math.max 0 (math.floor (/ (- columns 2) 2))))]
            (tset rails (+ (length rails) 1)
                  (span "▏ " (compose base :quote))))
          (append result (flow (inline-spans block.inlines base :quote) columns
                               rails rails)))
        (= block.kind :list_item)
        (let [raw-marker (or (and (not= block.checked nil)
                                  (or (and block.checked "☑ ") "☐ "))
                             (or (and block.ordered
                                      (.. (tostring (or block.number :1.)) " "))
                                 "• "))
              marker (if (> columns 2)
                         (misa.layout.take raw-marker (- columns 2))
                         "")
              indent (string.rep " "
                                 (math.min (* 2
                                              (math.max 0
                                                        (- (or block.depth 1) 1)))
                                           (math.max 0
                                                     (- columns
                                                        (misa.layout.width marker)
                                                        2))))
              first [(span indent base)
                     (span marker (compose base :markdown.list.marker))]
              rest [(span (.. indent
                              (string.rep " " (misa.layout.width marker)))
                          base)]]
          (append result (flow (inline-spans block.inlines base) columns first
                               rest)))
        (= block.kind :list_continuation)
        (let [prefix [(span (string.rep " "
                                        (math.min (* 2
                                                     (math.max 1
                                                               (or block.depth
                                                                   1)))
                                                  (math.max 0 (- columns 2))))
                            base)]]
          (append result (flow (inline-spans block.inlines base) columns prefix
                               prefix)))
        (= block.kind :thematic_rule)
        (tset result (+ (length result) 1)
              {:spans [(span (string.rep "─" columns)
                             (compose base :markdown.rule))]})
        (= block.kind :table)
        (append result (render-table block columns base))
        (= block.kind :code_block)
        (append result (code-block block columns base captures)))
    (each [_ line (ipairs result)]
      (set (line.source_start line.source_end)
           (values block.source_start block.source_end)))
    result))

(fn layout [document options previous]
  (set-forcibly! options (or options {}))
  (local base (or options.base :plain))
  (local columns (math.max 1
                           (math.min 512
                                     (math.floor (or (tonumber options.columns)
                                                     80)))))
  (local result {})
  (local entries [])
  (local base-key (style-key base))
  (each [index block (ipairs (or document.blocks {}))]
    (local captures
           (and options.captures (. options.captures block.source_start)))
    (var entry (and previous (. previous index)))
    (when (or (not entry) (not= entry.block block) (not= entry.columns columns)
              (or (not= entry.base base-key) (not= entry.captures captures)))
      (set entry
           {: block :base base-key
            : columns
            : captures
            :lines (render-block block columns base captures)}))
    (tset entries index entry)
    (append result entry.lines))
  (when (= (length result) 0)
    (tset result 1 {:spans [(span "" base)]}))
  (values result entries))

(fn render [document options]
  (local lines (layout document options))
  lines)

;; Reuse is explicit immutable input/output, not a hidden mutable document.
(fn project [text options previous]
  (local opts (or options {}))
  (local document (or opts.document
                      (and previous (= text previous.source) previous.document)
                      (misa.markdown.parse text (and previous previous.document))))
  (local base (style-key opts.base))
  (if (and previous (= document previous.document) (= text previous.source)
           (= opts.captures previous.captures) (= opts.columns previous.columns)
           (= base previous.base))
      previous
      (let [(lines entries) (layout document opts (and previous previous.entries))]
        {:source text : document :base base :columns opts.columns
         :captures opts.captures : entries : lines})))

(fn plain [text base]
  (var source (: (: (tostring (or text "")) :gsub "\r\n" "\n") :gsub "\r" "\n"))
  (when (= (source:sub -1) "\n") (set source (source:sub 1 -2)))
  (icollect [line (: (.. source "\n") :gmatch "(.-)\n")]
    {:spans [(span line base)]}))

{:setup (fn []
          (local setup-fx [])
          (assert (and misa.markdown (= (type misa.markdown.parse) :function))
                  "component.markdown requires markdown")
          (assert misa.layout "component.markdown requires layout")
          (table.insert setup-fx
                        {:type :register/service
                         :name :markdown_view
                         :value {: project
                                 : plain
                                 : render}})
          {:fx setup-fx})}
