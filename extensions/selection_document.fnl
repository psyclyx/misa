;; Source-preserving structural selection. Ranges are zero-based, half-open;

;; Markdown normalization is mapped back onto the original bytes before use.

(fn node [kind label first last children]
  {:children (or children {})
   : first
   : kind
   : label
   :last (math.max first last)})

(fn excerpt [source first last]
  (let [text (: (: (source:sub (+ first 1) last) :gsub "[\r\n]+" " ") :gsub
                "^%s+" "")
        clipped (misa.layout.clip text 48)]
    (.. clipped (or (and (< (length clipped) (length text)) "…") ""))))

(fn lines [source first last]
  (var (result at) (values {} first))
  (while (< at last)
    (local newline (source:find "[\r\n]" (+ at 1)))
    (local finish (math.min last (or (and newline (- newline 1)) last)))
    (tset result (+ (length result) 1)
          {:first at :last finish :text (source:sub (+ at 1) finish)})
    (when (or (not newline) (> newline last)) (lua :break))
    (set at newline)
    (when (= (source:sub newline (+ newline 1)) "\r\n")
      (set at (+ at 1))))
  result)

(fn source-map [source]
  (var (breaks at removed) (values {} 1 0))
  (while true
    (local found (source:find "\r\n" at true))
    (when (not found) (lua :break))
    (set removed (+ removed 1))
    (tset breaks (+ (length breaks) 1) (- (+ found 1) removed))
    (set at (+ found 2)))
  (fn [offset]
    (var (low high) (values 1 (length breaks)))
    (while (<= low high)
      (local middle (math.floor (/ (+ low high) 2)))
      (if (<= (. breaks middle) offset) (set low (+ middle 1))
          (set high (- middle 1))))
    (+ offset high)))

(fn table-rows [source first last]
  (let [rows {}]
    (each [row-index row (ipairs (lines source first last))]
      ;; A parsed table's second row is its alignment separator, never data.
      (when (not= row-index 2)
        (local (cells delimiters) (values {} {}))
        (local text row.text)
        (var (at code) (values 1 nil))
        (while (<= at (length text))
          (local char (text:sub at at))
          (if (and (= char "\\") (not code)) (set at (+ at 2)) (= char "`")
              (do
                (var finish at)
                (while (= (text:sub (+ finish 1) (+ finish 1)) "`")
                  (set finish (+ finish 1)))
                (local size (+ (- finish at) 1))
                (if (not code) (set code size)
                    (= code size) (set code nil))
                (set at (+ finish 1)))
              (do
                (when (and (= char "|") (not code))
                  (tset delimiters (+ (length delimiters) 1) at))
                (set at (+ at 1)))))
        (var start 0)

        (fn cell [finish]
          (let [content (text:sub (+ start 1) finish)
                leading (length (or (content:match "^%s*") ""))
                trailing (length (or (content:match "%s*$") ""))
                first-byte (+ row.first start leading)
                last-byte (math.max first-byte
                                    (- (+ row.first finish) trailing))]
            (tset cells (+ (length cells) 1)
                  (node :cell
                        (.. "Cell " (+ (length cells) 1) ": "
                            (excerpt source first-byte last-byte))
                        first-byte last-byte))
            nil))

        (each [index delimiter (ipairs delimiters)]
          (when (or (> index 1) (: (text:sub 1 (- delimiter 1)) :find "%S"))
            (cell (- delimiter 1)))
          (set start delimiter))
        (when (or (= (length delimiters) 0)
                  (: (text:sub (+ start 1)) :find "%S"))
          (cell (length text)))
        (tset rows (+ (length rows) 1)
              (node :row
                    (or (and (= (length rows) 0) "Header row")
                        (.. "Row " (length rows))) row.first
                    row.last cells))))
    rows))

{:setup (fn []
          (fn misa.selection_document [id label text]
            (local parsed (and misa.markdown (misa.markdown.parse text)))
            (local original (source-map text))
            (local root (node :message label 0 (length text)))
            (set (root.id root.text) (values id text))
            (local sections {})
            (var parent root)

            (fn close [until-level finish]
              (while (and (> (length sections) 0)
                          (>= (. sections (length sections) :level) until-level))
                (local section (table.remove sections))
                (set section.node.last (math.max section.node.first finish))
                (set section.content.last
                     (math.max section.content.first finish)))
              (set parent (or (and (> (length sections) 0)
                                   (. sections (length sections) :content))
                              root))
              nil)

            (each [_ block (ipairs (or (and parsed parsed.blocks) {}))]
              (local (first last)
                     (values (original block.source_start)
                             (original block.source_end)))
              (if (= block.kind :heading)
                  (do
                    (close block.level first)
                    (local title
                           (: (text:sub (+ first 1) last) :gsub "^%s*#+%s*" ""))
                    (local section (node :section title first (length text)))
                    (local heading
                           (node :heading (.. "Heading: " title) first last))
                    (var content-start last)
                    (if (= (text:sub (+ last 1) (+ last 2)) "\r\n")
                        (set content-start (+ last 2))
                        (: (text:sub (+ last 1) (+ last 1)) :match "[\r\n]")
                        (set content-start (+ last 1)))
                    (local content
                           (node :content "Section content" content-start
                                 (length text)))
                    (set section.children [heading content])
                    (tset parent.children (+ (length parent.children) 1)
                          section)
                    (tset sections (+ (length sections) 1)
                          {: content :level block.level :node section})
                    (set parent content))
                  (not= block.kind :blank)
                  (do
                    (local children
                           (or (and (= block.kind :table)
                                    (table-rows text first last))
                               nil))
                    (var label (block.kind:gsub "_" " "))
                    (when (not= block.kind :table)
                      (set label (.. label ": " (excerpt text first last))))
                    (tset parent.children (+ (length parent.children) 1)
                          (node block.kind label first last children)))))
            (close 0 (length text))
            root)

          ;; Fine selection is derived only when requested, not for every frame.

          (fn misa.selection_children [document current]
            (if (> (length current.children) 0) current.children
                (do
                  (local result {})
                  (local source document.text)
                  (local (first last) (values current.first current.last))
                  (if (= current.kind :character) result
                      (do
                        (if (= current.kind :word)
                            (do
                              (var at first)
                              (while (< at last)
                                (local finish
                                       (math.min last
                                                 (misa.layout.next_boundary source
                                                                            at)))
                                (tset result (+ (length result) 1)
                                      (node :character
                                            (source:sub (+ at 1) finish) at
                                            finish))
                                (set at finish)))
                            (or (or (= current.kind :line)
                                    (= current.kind :cell))
                                (= current.kind :heading))
                            (do
                              (local text (source:sub (+ first 1) last))
                              (var at 1)
                              (while (<= at (length text))
                                (local (a b) (text:find "%S+" at))
                                (when (not a) (lua :break))
                                (tset result (+ (length result) 1)
                                      (node :word (text:sub a b)
                                            (- (+ first a) 1) (+ first b)))
                                (set at (+ b 1))))
                            (each [_ line (ipairs (lines source first last))]
                              (tset result (+ (length result) 1)
                                    (node :line line.text line.first line.last))))
                        result)))))

          nil)}

