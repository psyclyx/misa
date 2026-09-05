;; Pure Markdown parsing. Semantic content has no size or nesting budget;
;; terminal layout, colors, and syntax highlighting belong to the view layer.

(fn trim [value] (: (value:gsub "^%s+" "") :gsub "%s+$" ""))

(fn marks-copy [marks mark]
  (local result [])
  (each [index value (ipairs marks)] (tset result index value))
  (when mark (table.insert result mark))
  result)

(local delimiters [["***" :strong_emphasis]
                   ["___" :strong_emphasis]
                   ["**" :strong]
                   ["__" :strong]
                   ["~~" :strikethrough]
                   ["*" :emphasis]
                   ["_" :emphasis]])

(fn parse-inlines [text marks]
  (local result [])
  (var plain [])
  ;; The cursor only advances. Remember the next terminator (or its absence),
  ;; so incomplete markup does not search the same suffix for every opener.
  (local endings {})

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
                     :text (table.concat plain)})
      (set plain [])))

  (fn emit [node]
    (emit-plain)
    (table.insert result node))

  (var at 1)
  (while (<= at (length text))
    (local special (text:find "[\\`%[%*_~]" at))
    (if (not= special at)
        (let [after (or special (+ (length text) 1))]
          (table.insert plain (text:sub at (- after 1)))
          (set at after))
        (let [char (text:sub at at)
              escaped (and (= char "\\") (text:sub (+ at 1) (+ at 1)))]
          (if (and escaped (not= escaped "") (escaped:match "[%p]"))
              (do
                (table.insert plain escaped)
                (set at (+ at 2)))
              (let [ticks (text:match "^(`+)" at)]
                (if ticks
                    (let [close (closing ticks (+ at (length ticks)))]
                      (if close
                          (do
                            (emit {:kind :code
                                   :marks (marks-copy marks)
                                   :text (text:sub (+ at (length ticks))
                                                   (- close 1))})
                            (set at (+ close (length ticks))))
                          (do
                            (table.insert plain ticks)
                            (set at (+ at (length ticks))))))
                    (let [label-end (and (= char "[") (closing "]" (+ at 1)))
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
                                                                  marks)})
                                  (set at (+ close 1)))
                                (do
                                  (table.insert plain char)
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
                                                     (+ at (length delimiter)))]
                                  (if (and close
                                           (> close (+ at (length delimiter))))
                                      (do
                                        (emit-plain)
                                        (each [_ child (ipairs (parse-inlines (text:sub (+ at
                                                                                           (length delimiter))
                                                                                        (- close
                                                                                           1))
                                                                              (marks-copy marks
                                                                                          mark)))]
                                          (table.insert result child))
                                        (set at (+ close (length delimiter))))
                                      (do
                                        (table.insert plain delimiter)
                                        (set at (+ at (length delimiter))))))
                                (do
                                  (table.insert plain char)
                                  (set at (+ at 1)))))))))))))
  (emit-plain)
  result)

(fn split-table [line]
  (if (not (line:find "|" 1 true)) nil
      (do
        (var (cells current) (values {} {}))
        (var (escaped in-code) (values false false))
        (for [at 1 (length line)]
          (local char (line:sub at at))
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
              (do
                (tset cells (+ (length cells) 1) (trim (table.concat current)))
                (set current {}))
              (tset current (+ (length current) 1) char)))
        (tset cells (+ (length cells) 1) (trim (table.concat current)))
        (when (= (: (trim line) :sub 1 1) "|")
          (table.remove cells 1))
        (when (= (: (trim line) :sub (- 1)) "|")
          (table.remove cells))
        (if (< (length cells) 1) nil cells))))

(fn table-separator [line]
  (let [cells (split-table line)]
    (if (not cells) nil (let [align {}]
                          (each [index cell (ipairs cells)]
                            (set-forcibly! cell (trim cell))
                            (when (or (not (cell:match "^:?-+:?$"))
                                      (< (select 2 (cell:gsub "-" "")) 3))
                              (lua "return nil"))
                            (tset align index
                                  (or (and (= (cell:sub 1 1) ":")
                                           (or (and (= (cell:sub (- 1)) ":")
                                                    :center)
                                               :left))
                                      (or (and (= (cell:sub (- 1)) ":") :right)
                                          :left))))
                          align))))

(fn thematic [line]
  (let [compact (: (trim line) :gsub "%s" "")]
    (if (< (length compact) 3)
        false
        (or (or (compact:match "^%*+$") (compact:match "^%-+$"))
            (compact:match "^_+$")))))

(fn make-parser []
  (let [parse-inline #(parse-inlines $ [])]
    (fn [value previous]
      (local raw-source (tostring (or value "")))
      (local source (if (raw-source:find "\r" 1 true)
                        (: (raw-source:gsub "\r\n" "\n") :gsub "\r" "\n")
                        raw-source))
      (if (and previous (= source previous.source))
          previous
          (do
            ;; Appending can change the final block and its predecessor (a partial
            ;; separator can become a table, or cease to terminate a paragraph). Earlier
            ;; blocks are immutable. Keep them, including their source coordinates and
            ;; list context, and run the very same parser over the uncertain suffix.
            (var (blocks start) (values {} 0))
            (local appended
                   (and previous
                        (= (source:sub 1 (length previous.source))
                           previous.source)))
            (when appended
              (local retained (math.max 0 (- (length previous.blocks) 2)))
              (for [i 1 retained] (tset blocks i (. previous.blocks i)))
              (set start (or (and (. previous.blocks (+ retained 1))
                                  (. previous.blocks (+ retained 1)
                                     :source_start))
                             0)))
            (local lines {})
            (each [line (: (.. (source:sub (+ start 1)) "\n") :gmatch "(.-)\n")]
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
              (local (source-index block-count) (values index (length blocks)))
              (local raw (. lines index))
              (var (fence info) (raw:match "^%s*(```+)%s*([^%s`]*)[^`]*$"))
              (when (not fence)
                (set (fence info) (raw:match "^%s*(~~~+)%s*([^%s~]*)[^~]*$")))
              (if fence
                  (let [body {}]
                    (set index (+ index 1))
                    (while (<= index (length lines))
                      (local marker (fence:sub 1 1))
                      (local closing
                             (: (. lines index) :match "^%s*([`~]+)%s*$"))
                      (when (and (and (and closing (= (closing:sub 1 1) marker))
                                      (not (closing:find (.. "[^" marker "]"))))
                                 (>= (length closing) (length fence)))
                        (lua :break))
                      (tset body (+ (length body) 1) (. lines index))
                      (set index (+ index 1)))
                    (when (<= index (length lines)) (set index (+ index 1)))
                    (add {:kind :code_block
                          :language (or (and (info:match "^[%w_+#.-]+$") info)
                                        "")
                          :text (table.concat body "\n")}))
                  (let [header (split-table raw)
                        alignment (or (and (< index (length lines))
                                           (table-separator (. lines
                                                               (+ index 1))))
                                      nil)]
                    (if (and (and header alignment)
                             (= (length header) (length alignment)))
                        (let [rows [{}]]
                          (each [_ cell (ipairs header)]
                            (tset (. rows 1) (+ (length (. rows 1)) 1)
                                  (parse-inline cell)))
                          (set index (+ index 2))
                          (while (<= index (length lines))
                            (local cells (split-table (. lines index)))
                            (when (not cells) (lua :break))
                            (local row {})
                            (for [column 1 (length alignment)]
                              (tset row column
                                    (parse-inline (or (. cells column) ""))))
                            (tset rows (+ (length rows) 1) row)
                            (set index (+ index 1)))
                          (add {:align alignment :kind :table : rows}))
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
                                      :kind :quote})
                                (set index (+ index 1)))
                              (or bullet ordered)
                              (do
                                (var checked nil)
                                (local (check item-body)
                                       (item:match "^%[([ xX])%]%s*(.*)$"))
                                (when check (set checked (not= check " "))
                                  (set item item-body))
                                (add {: checked
                                      :depth (+ (math.floor (/ (length indent)
                                                               2))
                                                1)
                                      :inlines (parse-inline item)
                                      :kind :list_item
                                      :number ordered
                                      :ordered (not= ordered nil)})
                                (set index (+ index 1)))
                              (and (and (raw:match "^%s+")
                                        (> (length blocks) 0))
                                   (or (= (. blocks (length blocks) :kind)
                                          :list_item)
                                       (= (. blocks (length blocks) :kind)
                                          :list_continuation)))
                              (let [(spaces body) (raw:match "^(%s+)(.*)$")]
                                (add {:depth (math.floor (/ (length spaces) 2))
                                      :inlines (parse-inline body)
                                      :kind :list_continuation})
                                (set index (+ index 1)))
                              (= raw "")
                              (do
                                (add {:kind :blank})
                                (set index (+ index 1)))
                              (let [parts [raw]]
                                (set index (+ index 1))
                                (while (and (and (and (and (and (and (and (and (<= index
                                                                                   (length lines))
                                                                               (not= (. lines
                                                                                        index)
                                                                                     ""))
                                                                          (not (: (. lines
                                                                                     index)
                                                                                  :match
                                                                                  "^%s*(#+)%s*")))
                                                                     (not (: (. lines
                                                                                index)
                                                                             :match
                                                                             "^%s*>")))
                                                                (not (: (. lines
                                                                           index)
                                                                        :match
                                                                        "^%s*[-+*]%s+")))
                                                           (not (: (. lines
                                                                      index)
                                                                   :match
                                                                   "^%s*%d+[.)]%s+")))
                                                      (not (: (. lines index)
                                                              :match "^%s*```")))
                                                 (not (: (. lines index) :match
                                                         "^%s*~~~")))
                                            (not (thematic (. lines index))))
                                  (when (table-separator (. lines index))
                                    (lua :break))
                                  (tset parts (+ (length parts) 1)
                                        (trim (. lines index)))
                                  (set index (+ index 1)))
                                (add {:inlines (parse-inline (table.concat parts
                                                                           " "))
                                      :kind :paragraph})))))))
              (for [block-index (+ block-count 1) (length blocks)]
                (tset (. blocks block-index) :source_start
                      (. offsets source-index))
                (tset (. blocks block-index) :source_end
                      (math.min (length source)
                                (- (or (. offsets index) (+ (length source) 1))
                                   1)))
                (local old (and appended (. previous.blocks block-index)))
                (local block (. blocks block-index))
                ;; A conservatively reparsed neighbor often remains unchanged. Keep its
                ;; identity too, so downstream layout/highlighting need not repeat.
                (when (and (and (and old (= old.kind block.kind))
                                (= old.source_start block.source_start))
                           (= old.source_end block.source_end))
                  (tset blocks block-index old))))
            {: blocks :kind :document : source})))))

{:setup (fn []
          (local setup-fx [])
          (assert (= misa.markdown nil) "markdown service already installed")
          (local parse (make-parser))
          (table.insert setup-fx
                        {:type :register/service
                         :name :markdown
                         :value {:new_document (fn []
                                                 (var (source document) nil)
                                                 {:update (fn [_ value]
                                                            (local next-source
                                                                   (tostring (or value
                                                                                 "")))
                                                            (when (not= next-source
                                                                        source)
                                                              (set document
                                                                   (parse next-source
                                                                          document))
                                                              (set source
                                                                   next-source))
                                                            document)})
                                 : parse}})
          {:fx setup-fx})}
