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
  (let [result []]
    (var at first)
    (while (< at last)
      (let [newline (source:find "[\r\n]" (+ at 1))
            finish (math.min last (or (and newline (- newline 1)) last))]
        (table.insert result
                      {:first at
                       :last finish
                       :text (source:sub (+ at 1) finish)})
        (set at (if (or (not newline) (> newline last)) last
                    (= (source:sub newline (+ newline 1)) "\r\n") (+ newline 1)
                    newline))))
    result))

(fn source-map [source]
  (let [breaks []]
    (each [found (source:gmatch "()\r\n")]
      (table.insert breaks (- found (length breaks))))
    (fn [offset]
      (var low 1)
      (var high (length breaks))
      (while (<= low high)
        (let [middle (math.floor (/ (+ low high) 2))]
          (if (<= (. breaks middle) offset) (set low (+ middle 1))
              (set high (- middle 1)))))
      (+ offset high))))

(fn table-rows [source first last]
  (let [rows {}]
    (each [row-index row (ipairs (lines source first last))]
      ;; A parsed table's second row is its alignment separator, never data.
      (when (not= row-index 2)
        (let [cells {}
              delimiters {}
              text row.text]
          (var (at code) (values 1 nil))
          (while (<= at (length text))
            (let [char (text:sub at at)]
              (if (and (= char "\\") (not code)) (set at (+ at 2)) (= char "`")
                  (do
                    (var finish at)
                    (while (= (text:sub (+ finish 1) (+ finish 1)) "`")
                      (set finish (+ finish 1)))
                    (let [size (+ (- finish at) 1)]
                      (if (not code) (set code size)
                          (= code size) (set code nil))
                      (set at (+ finish 1))))
                  (do
                    (when (and (= char "|") (not code))
                      (tset delimiters (+ (length delimiters) 1) at))
                    (set at (+ at 1))))))
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
                          (.. "Row " (length rows)))
                      row.first row.last cells)))))
    rows))

(fn selection-children [document current]
  "Return selectable children of a semantic document node."
  (if (> (length current.children) 0)
      current.children
      (let [result {}
            source document.text
            first current.first
            last current.last]
        (if (= current.kind :character) result
            (do
              (if (= current.kind :word)
                  (do
                    (var at first)
                    (while (< at last)
                      (let [finish (math.min last
                                             (misa.layout.next-boundary source
                                                                        at))]
                        (tset result (+ (length result) 1)
                              (node :character (source:sub (+ at 1) finish) at
                                    finish))
                        (set at finish))))
                  (or (= current.kind :line) (= current.kind :cell)
                      (= current.kind :heading))
                  (let [text (source:sub (+ first 1) last)]
                    (each [a word b (text:gmatch "()(%S+)()")]
                      (table.insert result
                                    (node :word word (- (+ first a) 1)
                                          (- (+ first b) 1)))))
                  (each [_ line (ipairs (lines source first last))]
                    (tset result (+ (length result) 1)
                          (node :line line.text line.first line.last))))
              result)))))

(fn selection-document [id label text]
  "Build a semantic selection document from source text."
  (let [parsed (and misa.markdown (misa.markdown.parse text))
        original (source-map text)
        root (node :message label 0 (length text))]
    (set (root.id root.text) (values id text))
    (let [sections {}]
      (var parent root)
      (let [lists []]
        (fn list-block [block first last]
          (let [depth (math.max 1 (or block.depth 1))]
            (while (and (> (length lists) 0)
                        (> (. lists (length lists) :depth) depth))
              (table.remove lists))
            (var frame (. lists (length lists)))
            (when (or (not frame) (< frame.depth depth))
              (let [list (node :list :List first last)]
                (table.insert (or (and frame frame.item.children)
                                  parent.children)
                              list)
                (set frame {: depth :node list})
                (table.insert lists frame)))
            (let [line (node :line (excerpt text first last) first last)]
              (if (and (= block.kind :list_continuation) frame.item)
                  (table.insert frame.item.children line)
                  (let [item (node :list_item
                                   (.. "Item: " (excerpt text first last)) first
                                   last [line])]
                    (table.insert frame.node.children item)
                    (set frame.item item)))
              (each [_ ancestor (ipairs lists)]
                (set ancestor.node.last last)
                (set ancestor.item.last last)))))

        (fn close [until-level finish]
          (while (and (> (length sections) 0)
                      (>= (. sections (length sections) :level) until-level))
            (let [section (table.remove sections)]
              (set section.node.last (math.max section.node.first finish))
              (set section.content.last (math.max section.content.first finish))))
          (set parent (or (and (> (length sections) 0)
                               (. sections (length sections) :content))
                          root))
          nil)

        (each [_ block (ipairs (or (and parsed parsed.blocks) {}))]
          (let [first (original block.source_start)
                last (original block.source_end)]
            (when (and (not= block.kind :list_item)
                       (not= block.kind :list_continuation)
                       (not= block.kind :blank))
              (while (> (length lists) 0) (table.remove lists)))
            (if (or (= block.kind :list_item) (= block.kind :list_continuation))
                (list-block block first last)
                (= block.kind :heading)
                (do
                  (close block.level first)
                  (let [title (: (text:sub (+ first 1) last) :gsub "^%s*#+%s*"
                                 "")
                        section (node :section title first (length text))
                        heading (node :heading (.. "Heading: " title) first
                                      last)]
                    (var content-start last)
                    (if (= (text:sub (+ last 1) (+ last 2)) "\r\n")
                        (set content-start (+ last 2))
                        (: (text:sub (+ last 1) (+ last 1)) :match "[\r\n]")
                        (set content-start (+ last 1)))
                    (let [content (node :content "Section content"
                                        content-start (length text))]
                      (set section.children [heading content])
                      (tset parent.children (+ (length parent.children) 1)
                            section)
                      (tset sections (+ (length sections) 1)
                            {: content :level block.level :node section})
                      (set parent content))))
                (not= block.kind :blank)
                (let [children (or (and (= block.kind :table)
                                        (table-rows text first last))
                                   nil)]
                  (var label (block.kind:gsub "_" " "))
                  (when (not= block.kind :table)
                    (set label (.. label ": " (excerpt text first last))))
                  (tset parent.children (+ (length parent.children) 1)
                        (node block.kind label first last children))))))
        (close 0 (length text))
        root))))

{: selection-children : selection-document}
