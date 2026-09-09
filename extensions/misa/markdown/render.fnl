(local definitions (require :misa.definitions))

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
      (let [token (select index ...)]
        (when token
          (tset result (+ (length result) 1) token))))
    result))

(fn style-key [base]
  (if (not= (type base) :table) (.. (type base) ":" (tostring base))
      (let [parts ["tokens:"]]
        (each [_ token (ipairs base)]
          (tset parts (+ (length parts) 1) (.. (length token) ":" token)))
        (table.concat parts))))

(fn span [text style link] {: link : style : text})

(fn inline-spans [nodes base extra origin]
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

              (fn emit [text first last]
                (let [item (span text style link)]
                  (set item.source true)
                  (set (item.source_start item.source_end) (values first last))
                  (table.insert result item)))

              (if (or (= origin nil) (= node.source_start nil))
                  (emit (or node.text "") nil nil)
                  (= (type origin) :number)
                  (emit (or node.text "") (+ origin node.source_start)
                        (+ origin node.source_end))
                  (do
                    (var (low high) (values 1 (length origin)))
                    (while (<= low high)
                      (let [middle (math.floor (/ (+ low high) 2))]
                        (if (<= (. origin middle :last) node.source_start)
                            (set low (+ middle 1))
                            (set high (- middle 1)))))
                    (do
                      (var finished? false)
                      (for [index low (length origin) &until finished?]
                        (let [run (. origin index)]
                          (when (>= run.first node.source_end)
                            (set finished? true))
                          (when (not finished?)
                            (let [first (math.max run.first node.source_start)
                                  last (math.min run.last node.source_end)]
                              (when (< first last)
                                (emit (node.text:sub (+ (- first
                                                           node.source_start)
                                                        1)
                                                     (- last node.source_start))
                                      (+ run.source (- first run.first))
                                      (+ run.source (- last run.first))))))))))))))
      nil)

    (visit nodes nil)
    (when (= (length result) 0)
      (tset result 1 (span "" (compose base extra))))
    result))

(fn flow [spans columns first-prefix rest-prefix options]
  (misa.layout.flow-spans spans columns first-prefix rest-prefix options))

(fn append [target lines]
  (each [_ line (ipairs lines)]
    (tset target (+ (length target) 1) line))
  nil)

(fn pad-spans [spans width alignment base]
  (var used 0)
  (each [_ item (ipairs spans)]
    (set used (+ used (misa.layout.width (or item.text "")))))
  (let [gap (math.max 0 (- width used))
        left (or (and (= alignment :right) gap)
                 (and (= alignment :center) (math.floor (/ gap 2))) 0)
        right (- gap left)
        result {}]
    (when (> left 0)
      (tset result (+ (length result) 1) (span (string.rep " " left) base)))
    (each [_ item (ipairs spans)]
      (tset result (+ (length result) 1) item))
    (when (> right 0)
      (tset result (+ (length result) 1) (span (string.rep " " right) base)))
    result))

(fn cell-lines [nodes width base header origin]
  (flow (inline-spans nodes base (or (and header :markdown.table.header) nil)
                      origin) (math.max 1 width) {} {}))

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
    (let [room (math.max (* 2 count) (- (- available (+ count 1)) (* 2 count)))]
      (var total 0)
      (each [_ width (ipairs widths)] (set total (+ total width)))
      (do
        (var finished? false)
        (while (and (not finished?) (> total room))
          (var widest 1)
          (for [column 2 count]
            (when (> (. widths column) (. widths widest)) (set widest column)))
          (when (<= (. widths widest) 2) (set finished? true))
          (when (not finished?)
            (tset widths widest (- (. widths widest) 1))
            (set total (- total 1)))))
      (do
        (var finished? false)
        (while (and (not finished?) (< total room))
          (var grew false)
          (for [column 1 count]
            (when (and (< total room) (< (. widths column) 40))
              (tset widths column (+ (. widths column) 1))
              (set total (+ total 1))
              (set grew true)))
          (when (not grew) (set finished? true))))
      widths)))

(fn border [widths left middle right base]
  (let [pieces [left]]
    (each [index width (ipairs widths)]
      (tset pieces (+ (length pieces) 1) (string.rep "─" (+ width 2)))
      (tset pieces (+ (length pieces) 1)
            (or (and (= index (length widths)) right) middle)))
    {:content false
     :spans [(span (table.concat pieces) (compose base :markdown.table.border))]}))

(fn render-table [block columns base]
  ;; A complete table needs borders, padding, and room for a wide grapheme
  ;; in every cell. Below that width, preserve its fields in a stacked view.
  (if (< columns (+ (* 5 (length block.align)) 1))
      (let [result {}
            header (or (. block.rows 1) {})]
        (for [row-index 2 (length block.rows)]
          (when (> row-index 2)
            (tset result (+ (length result) 1)
                  {:content false :spans [(span "" base)]}))
          (for [column 1 (length block.align)]
            (let [spans (inline-spans (or (. header column) {}) base
                                      :markdown.table.header
                                      (and block.cell_positions
                                           (. block.cell_positions 1 column)))]
              (tset spans (+ (length spans) 1) (span ": " base))
              (each [_ item (ipairs (inline-spans (or (. block.rows row-index
                                                         column)
                                                      {})
                                                  base nil
                                                  (and block.cell_positions
                                                       (. block.cell_positions
                                                          row-index column))))]
                (tset spans (+ (length spans) 1) item))
              (append result (flow spans columns)))))
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
                  (cell-lines (or (. row column) {}) width base (= row-index 1)
                              (and block.cell_positions
                                   (. block.cell_positions row-index column))))
            (set height (math.max height (length (. cells column)))))
          (for [line-index 1 height]
            (let [pieces [(span "│" (compose base :markdown.table.border))]]
              (each [column width (ipairs widths)]
                (tset pieces (+ (length pieces) 1) (span " " base))
                (let [content (. (or (. cells column line-index)
                                     {:spans [(span "" base)]})
                                 :spans)]
                  (each [_ item (ipairs (pad-spans content width
                                                   (. block.align column) base))]
                    (tset pieces (+ (length pieces) 1) item))
                  (tset pieces (+ (length pieces) 1)
                        (span " │" (compose base :markdown.table.border)))))
              (tset result (+ (length result) 1) {:spans pieces})))
          (when (< row-index (length block.rows))
            (tset result (+ (length result) 1)
                  (border widths "├" "┼" "┤" base))))
        (tset result (+ (length result) 1)
              (border widths "└" "┴" "┘" base))
        result)))

(fn highlighted-lines [block base captures]
  (let [source (or block.text "")
        captures (or captures {})]
    (var (spans at) (values {} 1))
    (each [_ capture (ipairs captures)]
      (let [first (+ (or (tonumber capture.start_byte) 0) 1)
            after (+ (or (tonumber capture.end_byte) 0) 1)
            class capture.capture]
        (when (and (. syntax-classes class) (>= first at) (> after first)
                   (<= after (+ (length source) 1)))
          (when (> first at)
            (tset spans (+ (length spans) 1)
                  (span (source:sub at (- first 1)) (compose base :code))))
          (tset spans (+ (length spans) 1)
                (span (source:sub first (- after 1))
                      (compose base :code (.. :syntax. class))))
          (set at after))))
    (when (<= at (length source))
      (tset spans (+ (length spans) 1)
            (span (source:sub at) (compose base :code))))
    (when (= (length spans) 0)
      (tset spans 1 (span source (compose base :code))))
    (let [lines [{:spans {}}]]
      (each [_ item (ipairs spans)]
        (var start 1)
        (do
          (var finished? false)
          (while (and (not finished?) true)
            (let [newline (item.text:find "\n" start true)]
              (when (not newline)
                (tset (. lines (length lines) :spans)
                      (+ (length (. lines (length lines) :spans)) 1)
                      (span (item.text:sub start) item.style item.link))
                (set finished? true))
              (when (not finished?)
                (tset (. lines (length lines) :spans)
                      (+ (length (. lines (length lines) :spans)) 1)
                      (span (item.text:sub start (- newline 1)) item.style
                            item.link))
                (tset lines (+ (length lines) 1) {:spans {}})
                (set start (+ newline 1)))))))
      (var offset block.content_start)
      (each [_ line (ipairs lines)]
        (each [_ item (ipairs line.spans)]
          (set item.source true)
          (when offset
            (set item.source_start offset)
            (set offset (+ offset (length item.text)))
            (set item.source_end offset)))
        (when offset (set offset (+ offset 1))))
      lines)))

(fn code-block [block columns base captures outer-inset]
  (let [language (or (and (not= block.language "") block.language) :plain)
        result {}
        source-lines (if block.rows
                         (icollect [_ row (ipairs block.rows)]
                           {:spans [{:text row.text
                                     :style (compose base :code)
                                     :source (not= row.source_start nil)
                                     :source_start row.source_start
                                     :source_end (when row.source_start
                                                   (+ row.source_start
                                                      (length row.text)))}]})
                         (highlighted-lines block base captures))]
    ;; A terminal newline ends the last line; only additional newlines are
    ;; blank content. Keep this policy shared by all tool code views.
    (when (and block.terminated (not block.rows)
               (= (: (or block.text "") :sub -1) "\n"))
      (table.remove source-lines))
    (when (and block.missing_newline (not block.rows)
               (not= (or block.text "") "") (not= (block.text:sub -1) "\n"))
      (table.insert source-lines
                    {:annotation true
                     :spans [{:text block.missing_newline
                              :style (compose base :code :dim)
                              :source false}]}))
    (var digits (length (tostring (length source-lines))))
    (each [_ row (ipairs (or block.rows []))]
      (set digits (math.max digits (length (tostring (or row.number ""))))))
    (let [diff (= language :diff)]
      (when diff
        (each [_ line (ipairs source-lines)]
          (let [raw (table.concat (icollect [_ part (ipairs line.spans)]
                                    part.text))
                (old count-old new count-new) (raw:match "^@@ %-(%d+),?(%d*) %+(%d+),?(%d*) @@")]
            (when old
              (set digits
                   (math.max digits
                             (length (tostring (+ (tonumber old)
                                                  (or (tonumber count-old) 1))))
                             (length (tostring (+ (tonumber new)
                                                  (or (tonumber count-new) 1))))))))))
      (var (old-line new-line) (values 1 1))
      (each [index line (ipairs source-lines)]
        (let [raw (table.concat (icollect [_ item (ipairs line.spans)]
                                  item.text))
              (old-start new-start) (raw:match "^@@ %-(%d+)[^ ]* %+(%d+)[^ ]* @@")
              marker (raw:sub 1 1)
              metadata (or old-start (raw:match "^diff ") (raw:match "^index ")
                           (raw:match "^%-%-%- ") (raw:match "^%+%+%+ ")
                           (= marker "\\"))]
          (when old-start
            (set (old-line new-line)
                 (values (tonumber old-start) (tonumber new-start))))
          (var number (if block.rows
                          (tostring (or (. block.rows index :number) ""))
                          (tostring index)))
          (when line.annotation (set number ""))
          (when diff
            ;; One gutter: removed lines refer to the old file; context and
            ;; additions refer to the new file. The diff marker supplies the side.
            (set number
                 (if metadata ""
                     (tostring (if (= marker "-") old-line new-line))))
            (when (not metadata)
              (each [_ item (ipairs line.spans)]
                (set item.style
                     (compose base :code
                              (if (= marker "+") :diff.added
                                  (= marker "-") :diff.removed
                                  :plain))))
              (when (not= marker "+") (set old-line (+ old-line 1)))
              (when (not= marker "-") (set new-line (+ new-line 1)))))
          ;; Surrounding margins inherit the parent; gutter and code share a surface.
          (set number (.. (string.rep " "
                                      (math.max 0 (- digits (length number))))
                          number))
          (let [numbered (not= block.numbered false)
                requested-left (math.max 0 (- 3 (or outer-inset 0)))
                left (if (> columns (+ requested-left 4)) requested-left 0)
                right (if (> columns (+ requested-left 4)) 3 0)
                gutter (if (and numbered
                                (> (- columns left right) (+ (length number) 3)))
                           (.. number "  ")
                           "")
                body-width (math.max 1 (- columns left right (length gutter)))]
            (each [wrapped-index row (ipairs (flow line.spans body-width nil
                                                   nil
                                                   {:trim false :words false}))]
              (let [parts [{:text (string.rep " " left)
                            :style base
                            :source false}
                           {:text (if (= wrapped-index 1) gutter
                                      (string.rep " " (length gutter)))
                            :style [:dim :surface.code]
                            :source false}]]
                (var used 0)
                (each [_ item (ipairs row.spans)]
                  (set used (+ used (misa.layout.width item.text)))
                  (table.insert parts
                                (misa.patch item
                                            {:style (compose item.style
                                                             :surface.code)})))
                (when (< used body-width)
                  (table.insert parts
                                {:text (string.rep " " (- body-width used))
                                 :style :surface.code
                                 :source false}))
                (when (> right 0)
                  (table.insert parts
                                {:text (string.rep " " right)
                                 :style base
                                 :source false}))
                (table.insert result
                              (misa.patch row
                                          {:spans (misa.replace parts)
                                           :annotation line.annotation})))))))
      result)))

(fn render-block [block columns base captures outer-inset]
  (let [result {}
        origin (or block.inline_map block.inline_start block.source_start)]
    (if (= block.kind :blank)
        (tset result (+ (length result) 1)
              {:content false :spans [(span "" base)]})
        (= block.kind :paragraph)
        (append result (flow (inline-spans block.inlines base nil origin)
                             columns))
        (= block.kind :heading)
        (let [token (or (. heading-marks block.level) (. heading-marks 6))
              prefix [{:text (or (. heading-prefix block.level) "· ")
                       :style (compose base token)
                       :selection_marker true
                       :source_start block.source_start
                       :source_end block.inline_start}]]
          (append result
                  (flow (inline-spans block.inlines base token origin) columns
                        prefix [(span "  " base)])))
        (= block.kind :quote)
        (let [rails {}]
          ;; Preserve semantic depth while leaving room for a wide grapheme.
          (for [_ 1 (math.min (math.max 1 (or block.depth 1))
                              (math.max 0 (math.floor (/ (- columns 2) 2))))]
            (tset rails (+ (length rails) 1)
                  {:text "▏ "
                   :style (compose base :quote)
                   :selection_marker true
                   :source_start block.source_start
                   :source_end block.inline_start}))
          (append result (flow (inline-spans block.inlines base :quote origin)
                               columns rails rails)))
        (= block.kind :list_item)
        (let [raw-marker (or (and (not= block.checked nil)
                                  (or (and block.checked "☑ ") "☐ "))
                             (and block.ordered
                                  (.. (tostring (or block.number :1.)) " "))
                             "• ")
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
                     {:text marker
                      :style (compose base :markdown.list.marker)
                      :selection_marker true
                      :source_start block.source_start
                      :source_end block.inline_start}]
              rest [(span (.. indent
                              (string.rep " " (misa.layout.width marker)))
                          base)]]
          (append result (flow (inline-spans block.inlines base nil origin)
                               columns first rest)))
        (= block.kind :list_continuation)
        (let [prefix [(span (string.rep " "
                                        (math.min (* 2
                                                     (math.max 1
                                                               (or block.depth
                                                                   1)))
                                                  (math.max 0 (- columns 2))))
                            base)]]
          (append result (flow (inline-spans block.inlines base nil origin)
                               columns prefix prefix)))
        (= block.kind :thematic_rule)
        (tset result (+ (length result) 1)
              {:spans [{:text (string.rep "─" columns)
                        :style (compose base :markdown.rule)
                        :source true
                        :source_start block.source_start
                        :source_end block.source_end}]})
        (= block.kind :table)
        (append result (render-table block columns base))
        (= block.kind :code_block)
        (append result (code-block block columns base captures outer-inset)))
    (each [_ line (ipairs result)]
      (set (line.source_start line.source_end)
           (values block.source_start block.source_end)))
    result))

(fn layout [document options previous]
  (let [options (or options {})
        base (or options.base :plain)
        columns (math.max 1
                          (math.min 512
                                    (math.floor (or (tonumber options.columns)
                                                    80))))
        result {}
        entries []
        base-key (style-key base)]
    (each [index block (ipairs (or document.blocks {}))]
      (let [captures (and options.captures
                          (. options.captures block.source_start))]
        (var entry (and previous (. previous index)))
        (when (or (not entry) (not= entry.block block)
                  (not= entry.columns columns)
                  (not= entry.outer_inset options.outer_inset)
                  (not= entry.base base-key) (not= entry.captures captures))
          (set entry
               {: block
                :base base-key
                : columns
                : captures
                :outer_inset options.outer_inset
                :lines (render-block block columns base captures
                                     options.outer_inset)}))
        (tset entries index entry)
        (append result entry.lines)))
    (when (= (length result) 0)
      (tset result 1 {:spans [(span "" base)]}))
    (values result entries)))

(fn render [document options]
  "Render a Markdown document into terminal lines."
  (let [lines (layout document options)]
    lines))

;; Reuse is explicit immutable input/output, not a hidden mutable document.
(fn project [text options previous]
  "Project a Markdown document while reusing unchanged block geometry."
  (let [opts (or options {})
        document (or opts.document
                     (and previous (= text previous.source) previous.document)
                     (misa.markdown.parse text (and previous previous.document)))
        base (style-key opts.base)]
    (if (and previous (= document previous.document) (= text previous.source)
             (= opts.captures previous.captures)
             (= opts.columns previous.columns)
             (= opts.outer_inset previous.outer_inset) (= base previous.base))
        previous
        (let [(lines entries) (layout document opts
                                      (and previous previous.entries))]
          {:source text
           : document
           :base base
           :columns opts.columns
           :captures opts.captures
           :outer_inset opts.outer_inset
           : entries
           : lines}))))

(fn plain [text base]
  "Render text as plain terminal lines with source coordinates."
  (var source (: (: (tostring (or text "")) :gsub "\r\n" "\n") :gsub "\r" "\n"))
  (when (= (source:sub -1) "\n") (set source (source:sub 1 -2)))
  (var offset 0)
  (icollect [line (: (.. source "\n") :gmatch "(.-)\n")]
    (let [first offset]
      (set offset (+ offset (length line) 1))
      {:spans [{:text line
                :style base
                :source true
                :source_start first
                :source_end (- offset 1)}]})))

(fn build []
  "Build the module declarations."
  (definitions.build :component.markdown
    [{:catalog :services
      :id :markdown.view
      :value {: project : plain : render}}]
    {:requirements {:component.markdown [:layout :markdown :markdown.parse]}}))

{:build build}
