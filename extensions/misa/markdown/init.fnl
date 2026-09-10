;; Pure Markdown parsing. Semantic content has no size or nesting budget;
;; terminal layout, colors, and syntax highlighting belong to the view layer.

(fn trim [value] (: (value:gsub "^%s+" "") :gsub "%s+$" ""))

(fn marks-copy [marks mark]
  (let [result []]
    (each [index value (ipairs marks)] (tset result index value))
    (when mark (table.insert result mark))
    result))

(local delimiters [["***" :strong_emphasis]
                   ["___" :strong_emphasis]
                   ["**" :strong]
                   ["__" :strong]
                   ["~~" :strikethrough]
                   ["*" :emphasis]
                   ["_" :emphasis]])

(fn parse-inlines [text marks base]
  (let [base (or base 0)
        result []]
    (var plain [])
    (var (plain-start plain-end) nil)

    (fn literal [value at]
      (when (= (length plain) 0) (set plain-start (- at 1)))
      (set plain-end (+ (- at 1) (length value)))
      (table.insert plain value))

    ;; The cursor only advances. Remember the next terminator (or its absence),
    ;; so incomplete markup does not search the same suffix for every opener.
    (let [endings {}]
      (fn closing [delimiter from]
        (var found (. endings delimiter))
        (when (or (= found nil) (and found (< found from)))
          (set found (or (text:find delimiter from true) false))
          (tset endings delimiter found))
        (if found found nil))

      (fn emit-plain []
        (when (> (length plain) 0)
          (table.insert result
                        {:kind :text
                         :marks (marks-copy marks)
                         :source_start (+ base plain-start)
                         :source_end (+ base plain-end)
                         :text (table.concat plain)})
          (set plain [])))

      (fn emit [node]
        (emit-plain)
        (table.insert result node))

      (var at 1)
      (while (<= at (length text))
        (let [special (text:find "[\\`%[%*_~]" at)]
          (if (not= special at)
              (let [after (or special (+ (length text) 1))]
                (literal (text:sub at (- after 1)) at)
                (set at after))
              (let [char (text:sub at at)
                    escaped (and (= char "\\") (text:sub (+ at 1) (+ at 1)))]
                (if (and escaped (not= escaped "") (escaped:match "[%p]"))
                    (do
                      (emit {:kind :text
                             :marks (marks-copy marks)
                             :text escaped
                             :source_start (+ base at)
                             :source_end (+ base at 1)})
                      (set at (+ at 2)))
                    (let [ticks (text:match "^(`+)" at)]
                      (if ticks
                          (let [close (closing ticks (+ at (length ticks)))]
                            (if close
                                (do
                                  (emit {:kind :code
                                         :source_start (+ base (- at 1)
                                                          (length ticks))
                                         :source_end (+ base (- close 1))
                                         :marks (marks-copy marks)
                                         :text (text:sub (+ at (length ticks))
                                                         (- close 1))})
                                  (set at (+ close (length ticks))))
                                (do
                                  (literal ticks at)
                                  (set at (+ at (length ticks))))))
                          (let [label-end (and (= char "[")
                                               (closing "]" (+ at 1)))
                                open (and label-end (+ label-end 1))]
                            (if (and open (= (text:sub open open) "("))
                                (let [close (closing ")" (+ open 1))
                                      target (if close
                                                 (trim (text:sub (+ open 1)
                                                                 (- close 1)))
                                                 "")
                                      safe (and (not= target "")
                                                (not (target:find "[%z\001-\031\127-\159]")))]
                                  (if (and close safe)
                                      (do
                                        (emit {:kind :link
                                               : target
                                               :children (parse-inlines (text:sub (+ at
                                                                                     1)
                                                                                  (- label-end
                                                                                     1))
                                                                        marks
                                                                        (+ base
                                                                           at))})
                                        (set at (+ close 1)))
                                      (do
                                        (literal char at)
                                        (set at (+ at 1)))))
                                (let [(delimiter mark) (accumulate [(delimiter mark) nil _ choice (ipairs delimiters)
                                                                    &until delimiter]
                                                         (when (= (text:sub at
                                                                            (- (+ at
                                                                                  (length (. choice
                                                                                             1)))
                                                                               1))
                                                                  (. choice 1))
                                                           (values (. choice 1)
                                                                   (. choice 2))))]
                                  (if delimiter
                                      (let [close (closing delimiter
                                                           (+ at
                                                              (length delimiter)))]
                                        (if (and close
                                                 (> close
                                                    (+ at (length delimiter))))
                                            (do
                                              (emit-plain)
                                              (each [_ child (ipairs (parse-inlines (text:sub (+ at
                                                                                                 (length delimiter))
                                                                                              (- close
                                                                                                 1))
                                                                                    (marks-copy marks
                                                                                                mark)
                                                                                    (+ base
                                                                                       (- at
                                                                                          1)
                                                                                       (length delimiter))))]
                                                (table.insert result child))
                                              (set at
                                                   (+ close (length delimiter))))
                                            (do
                                              (literal delimiter at)
                                              (set at (+ at (length delimiter))))))
                                      (do
                                        (literal char at)
                                        (set at (+ at 1))))))))))))))
      (emit-plain)
      result)))

(fn split-table [line]
  (if (not (line:find "|" 1 true)) nil
      (do
        (var (cells current positions start) (values {} {} {} 0))
        (var (escaped in-code) (values false false))
        (for [at 1 (length line)]
          (let [char (line:sub at at)]
            (if escaped (do
                          (tset current (+ (length current) 1) char)
                          (set escaped false))
                (= char "\\")
                (do
                  (set escaped true)
                  (tset current (+ (length current) 1) char))
                (= char "`")
                (do
                  (set in-code (not in-code))
                  (tset current (+ (length current) 1) char))
                (and (= char "|") (not in-code))
                (let [content (table.concat current)]
                  (table.insert positions
                                (+ start
                                   (length (or (content:match "^%s*") ""))))
                  (tset cells (+ (length cells) 1) (trim content))
                  (set (current start) (values {} at)))
                (tset current (+ (length current) 1) char))))
        (let [content (table.concat current)]
          (table.insert positions
                        (+ start (length (or (content:match "^%s*") ""))))
          (tset cells (+ (length cells) 1) (trim content))
          (when (= (: (trim line) :sub 1 1) "|")
            (table.remove cells 1)
            (table.remove positions 1))
          (when (= (: (trim line) :sub (- 1)) "|")
            (table.remove cells)
            (table.remove positions))
          (if (< (length cells) 1) nil (values cells positions))))))

(fn table-separator [line]
  (let [cells (split-table line)]
    (when (and cells
               (accumulate [valid? true _ value (ipairs cells)
                            &until (not valid?)]
                 (let [cell (trim value)]
                   (and (cell:match "^:?-+:?$")
                        (>= (select 2 (cell:gsub "-" "")) 3)))))
      (icollect [_ value (ipairs cells)]
        (let [cell (trim value)]
          (if (= (cell:sub -1) ":")
              (if (= (cell:sub 1 1) ":") :center :right)
              :left))))))

(fn thematic [line]
  (let [compact (: (trim line) :gsub "%s" "")]
    (if (< (length compact) 3) false
        (or (compact:match "^%*+$") (compact:match "^%-+$")
            (compact:match "^_+$")))))

(fn parse [value previous]
  "Parse Markdown while retaining unchanged blocks from the preceding document."
  (let [parse-inline #(parse-inlines $ [])
        raw-source (tostring (or value ""))
        source (if (raw-source:find "\r" 1 true)
                   (: (raw-source:gsub "\r\n" "\n") :gsub "\r" "\n")
                   raw-source)]
    (if (and previous (= source previous.source))
        previous
        (do
          ;; Appending can change the final block and its predecessor (a partial
          ;; separator can become a table, or cease to terminate a paragraph). Earlier
          ;; blocks are immutable. Keep them, including their source coordinates and
          ;; list context, and run the very same parser over the uncertain suffix.
          (var (blocks start) (values {} 0))
          (let [appended (and previous
                              (= (source:sub 1 (length previous.source))
                                 previous.source))]
            (when appended
              (let [retained (math.max 0 (- (length previous.blocks) 2))]
                (for [i 1 retained] (tset blocks i (. previous.blocks i)))
                (set start (or (and (. previous.blocks (+ retained 1))
                                    (. previous.blocks (+ retained 1)
                                       :source_start))
                               0))))
            (let [lines {}]
              (each [line (: (.. (source:sub (+ start 1)) "\n") :gmatch
                             "(.-)\n")]
                (tset lines (+ (length lines) 1) line))
              (when (= (source:sub (- 1)) "\n")
                (table.remove lines))
              (var (offsets offset) (values {} start))
              (each [i line (ipairs lines)]
                (tset offsets i offset)
                (set offset (+ offset (length line) 1)))
              (var index 1)

              (fn add [block] (table.insert blocks block))

              (while (<= index (length lines))
                (let [(source-index block-count) (values index (length blocks))
                      raw (. lines index)]
                  (var (fence info) (raw:match "^%s*(```+)%s*([^%s`]*)[^`]*$"))
                  (when (not fence)
                    (set (fence info)
                         (raw:match "^%s*(~~~+)%s*([^%s~]*)[^~]*$")))
                  (if fence
                      (let [body {}
                            content-start (or (. offsets (+ index 1))
                                              (length source))]
                        (var closing-start nil)
                        (set index (+ index 1))
                        (do
                          (var finished? false)
                          (while (and (not finished?) (<= index (length lines)))
                            (let [marker (fence:sub 1 1)
                                  closing (: (. lines index) :match
                                             "^%s*([`~]+)%s*$")]
                              (when (and closing (= (closing:sub 1 1) marker)
                                         (not (closing:find (.. "[^" marker "]")))
                                         (>= (length closing) (length fence)))
                                (set closing-start (. offsets index))
                                (set finished? true))
                              (when (not finished?)
                                (tset body (+ (length body) 1) (. lines index))
                                (set index (+ index 1))))))
                        (when (<= index (length lines))
                          (set index (+ index 1)))
                        (add {:kind :code_block
                              :content_start content-start
                              :closing_start closing-start
                              :language (or (and (info:match "^[%w_+#.-]+$")
                                                 info)
                                            "")
                              :text (table.concat body "\n")}))
                      (let [(header header-positions) (split-table raw)
                            alignment (or (and (< index (length lines))
                                               (table-separator (. lines
                                                                   (+ index 1))))
                                          nil)]
                        (if (and header alignment
                                 (= (length header) (length alignment)))
                            (let [rows [{}]
                                  positions [{}]]
                              (each [column cell (ipairs header)]
                                (tset (. positions 1) column
                                      (+ (. offsets index)
                                         (. header-positions column)))
                                (tset (. rows 1) (+ (length (. rows 1)) 1)
                                      (parse-inline cell)))
                              (set index (+ index 2))
                              (do
                                (var finished? false)
                                (while (and (not finished?)
                                            (<= index (length lines)))
                                  (let [(cells cell-positions) (split-table (. lines
                                                                               index))]
                                    (when (not cells) (set finished? true))
                                    (when (not finished?)
                                      (let [row {}
                                            row-positions {}]
                                        (for [column 1 (length alignment)]
                                          (tset row-positions column
                                                (+ (. offsets index)
                                                   (or (. cell-positions column)
                                                       0)))
                                          (tset row column
                                                (parse-inline (or (. cells
                                                                     column)
                                                                  ""))))
                                        (table.insert positions row-positions)
                                        (tset rows (+ (length rows) 1) row)
                                        (set index (+ index 1)))))))
                              (add {:align alignment
                                    :kind :table
                                    : rows
                                    :cell_positions positions}))
                            (let [(hashes heading) (raw:match "^%s*(#+)%s*(.-)%s*$")
                                  quote-body (raw:match "^%s*>%s?(.*)$")]
                              (var (indent bullet item)
                                   (raw:match "^(%s*)([-+*])%s+(.+)$"))
                              (var ordered nil)
                              (when (not bullet)
                                (set (indent ordered item)
                                     (raw:match "^(%s*)(%d+[.)])%s+(.+)$")))
                              (if (and hashes (<= (length hashes) 6))
                                  (do
                                    (add {:inlines (parse-inline heading)
                                          :inline_start (+ (. offsets index)
                                                           (- (or (raw:match "^%s*#+%s*()")
                                                                  1)
                                                              1))
                                          :kind :heading
                                          :level (length hashes)})
                                    (set index (+ index 1)))
                                  (thematic raw)
                                  (do
                                    (add {:kind :thematic_rule})
                                    (set index (+ index 1)))
                                  quote-body
                                  (do
                                    (var (depth at) (values 0 1))
                                    (var after (raw:match "^%s*>%s?()" at))
                                    (while after
                                      (set depth (+ depth 1))
                                      (set at after)
                                      (set after (raw:match "^%s*>%s?()" at)))
                                    (add {: depth
                                          :inlines (parse-inline (raw:sub at))
                                          :inline_start (+ (. offsets index)
                                                           (- at 1))
                                          :kind :quote})
                                    (set index (+ index 1)))
                                  (or bullet ordered)
                                  (do
                                    (var checked nil)
                                    (let [(check item-body) (item:match "^%[([ xX])%]%s*(.*)$")]
                                      (when check
                                        (set checked (not= check " "))
                                        (set item item-body))
                                      (add {: checked
                                            :depth (+ (math.floor (/ (length indent)
                                                                     2))
                                                      1)
                                            :inlines (parse-inline item)
                                            :inline_start (+ (. offsets index)
                                                             (- (length raw)
                                                                (length item)))
                                            :kind :list_item
                                            :number ordered
                                            :ordered (not= ordered nil)})
                                      (set index (+ index 1))))
                                  (and (raw:match "^%s+") (> (length blocks) 0)
                                       (or (= (. blocks (length blocks) :kind)
                                              :list_item)
                                           (= (. blocks (length blocks) :kind)
                                              :list_continuation)))
                                  (let [(spaces body) (raw:match "^(%s+)(.*)$")]
                                    (add {:depth (math.floor (/ (length spaces)
                                                                2))
                                          :inlines (parse-inline body)
                                          :inline_start (+ (. offsets index)
                                                           (length spaces))
                                          :kind :list_continuation})
                                    (set index (+ index 1)))
                                  (= raw "")
                                  (do
                                    (add {:kind :blank})
                                    (set index (+ index 1)))
                                  (let [parts [raw]
                                        mapping [{:first 0
                                                  :last (length raw)
                                                  :source (. offsets index)}]]
                                    (var flattened (length raw))
                                    (set index (+ index 1))
                                    (do
                                      (var finished? false)
                                      (while (and (not finished?)
                                                  (and (<= index (length lines))
                                                       (not= (. lines index) "")
                                                       (not (: (. lines index)
                                                               :match
                                                               "^%s*(#+)%s*"))
                                                       (not (: (. lines index)
                                                               :match "^%s*>"))
                                                       (not (: (. lines index)
                                                               :match
                                                               "^%s*[-+*]%s+"))
                                                       (not (: (. lines index)
                                                               :match
                                                               "^%s*%d+[.)]%s+"))
                                                       (not (: (. lines index)
                                                               :match "^%s*```"))
                                                       (not (: (. lines index)
                                                               :match "^%s*~~~"))
                                                       (not (thematic (. lines
                                                                         index)))))
                                        (when (table-separator (. lines index))
                                          (set finished? true))
                                        (when (not finished?)
                                          (let [next-line (. lines index)
                                                content (trim next-line)]
                                            (table.insert mapping
                                                          {:first flattened
                                                           :last (+ flattened 1)
                                                           :source (- (. offsets
                                                                         index)
                                                                      1)})
                                            (set flattened (+ flattened 1))
                                            (table.insert mapping
                                                          {:first flattened
                                                           :last (+ flattened
                                                                    (length content))
                                                           :source (+ (. offsets
                                                                         index)
                                                                      (length (or (next-line:match "^%s*")
                                                                                  "")))})
                                            (set flattened
                                                 (+ flattened (length content)))
                                            (tset parts (+ (length parts) 1)
                                                  content)
                                            (set index (+ index 1))))))
                                    (add {:inlines (parse-inline (table.concat parts
                                                                               " "))
                                          :inline_map mapping
                                          :kind :paragraph})))))))
                  (for [block-index (+ block-count 1) (length blocks)]
                    (tset (. blocks block-index) :source_start
                          (. offsets source-index))
                    (tset (. blocks block-index) :source_end
                          (math.min (length source)
                                    (- (or (. offsets index)
                                           (+ (length source) 1))
                                       1)))
                    (let [old (and appended (. previous.blocks block-index))
                          block (. blocks block-index)]
                      ;; A conservatively reparsed neighbor often remains unchanged. Keep its
                      ;; identity too, so downstream layout/highlighting need not repeat.
                      (when (and old (= old.kind block.kind)
                                 (= old.source_start block.source_start)
                                 (= old.source_end block.source_end))
                        (tset blocks block-index old))))))
              {: blocks :kind :document : source}))))))

(fn new-document []
  "Create a mutable convenience wrapper around incremental Markdown parsing."
  (var document nil)
  {:update (fn [_ value]
             (set document (parse value document))
             document)})

{: new-document : parse}
