(fn selection-key [selected]
  (when selected {:id selected.id :first selected.first :last selected.last}))

(fn same-selection? [a b]
  (and a b (= a.id b.id) (= a.first b.first) (= a.last b.last)))

(fn line-anchor [line]
  (when line
    (var (source last) nil)
    (each [_ span (ipairs (or line.spans []))]
      (when (and (or span.source span.selection_marker) span.source_start
                 span.source_end)
        (set source (math.min (or source span.source_start) span.source_start))
        (set last (math.max (or last span.source_end) span.source_end))))
    {:id line.transcript_id
     :part (or line.source_part :body)
     :source (or source line.source_start -1)
     : last
     :offset 0}))

(fn same-source? [a b]
  (and a b (= a.id b.id) (= a.part b.part) (= a.source b.source)))

;; Geometry access. `offsets` records the first row of each item, so a row maps
;; back to its item by binary search and a window never walks the transcript.

(fn item-at [layout row]
  "Return the index, item, and first content row covering a row."
  (let [items layout.items
        offsets layout.offsets
        count (length items)]
    (var (low high found) (values 1 count nil))
    (while (<= low high)
      (let [middle (math.floor (/ (+ low high) 2))]
        (if (<= (. offsets middle) row)
            (set (found low) (values middle (+ middle 1)))
            (set high (- middle 1)))))
    (when found
      (let [item (. items found)]
        (values found item (+ (. offsets found) (if item.space_before 1 0)))))))

(fn row-line [layout row]
  "Return the line rendered at a row, or nil for a spacing or attachment row."
  (let [(index item content) (item-at layout row)]
    (when item
      (let [offset (- row content)]
        (when (and (>= offset 0) (< offset (length item.lines)))
          (. item.lines (+ offset 1)))))))

(fn line-anchor-at [layout row]
  (line-anchor (row-line layout row)))

(fn anchor-at [layout row]
  "Anchor the row `row`, counting the rows of the same source before it."
  (let [anchor (line-anchor-at layout row)]
    (when anchor
      (var finished? false)
      (var previous (- row 1))
      (while (and (not finished?) (>= previous 1))
        (when (not (same-source? anchor (line-anchor-at layout previous)))
          (set finished? true))
        (when (not finished?)
          (set anchor.offset (+ anchor.offset 1))
          (set previous (- previous 1)))))
    anchor))

(fn anchored-row [layout anchor fallback]
  "Return the row whose source matches an anchor, preferring an exact offset."
  (var (found distance offset-distance) (values nil math.huge math.huge))
  (var (exact exact-offset) (values nil math.huge))
  (when anchor
    (each [index item (ipairs layout.items)]
      (when (= item.id anchor.id)
        (each [offset line (ipairs item.lines)]
          (let [candidate (line-anchor line)]
            (when (and candidate (= candidate.id anchor.id)
                       (or (not anchor.part) (= candidate.part anchor.part)))
              (let [row (+ (. layout.offsets index) (if item.space_before 1 0)
                           (- offset 1))
                    delta (if (and candidate.last
                                   (<= candidate.source anchor.source)
                                   (< anchor.source candidate.last))
                              0
                              (math.abs (- candidate.source anchor.source)))
                    offset-delta (math.abs (- candidate.offset anchor.offset))]
                (when (and (= candidate.source anchor.source)
                           (< offset-delta exact-offset))
                  (set (exact exact-offset) (values row offset-delta)))
                (when (or (< delta distance)
                          (and (= delta distance)
                               (< offset-delta offset-distance)))
                  (set (found distance offset-distance)
                       (values row delta offset-delta))))))))))
  (or exact found fallback))

(fn same-anchor? [a b]
  (and a b (= a.id b.id) (= a.part b.part) (= a.source b.source)
       (= a.offset b.offset)))

(fn viewport-top [messages layout bottom]
  (if (not messages.top) bottom
      ;; Scrolling owns a physical row. Only relocate its content when layout
      ;; changes have actually displaced that row; ordinary renders must not
      ;; reinterpret an explicit scroll as a request to find the block again.
      (or (not messages.anchor)
          (same-anchor? (anchor-at layout messages.top) messages.anchor))
      messages.top (anchored-row layout messages.anchor messages.top)))

(fn viewport [db context available-lines]
  "Select visible transcript rows and their source anchor."
  (let [room (math.max 0 (math.floor (or available-lines 0)))]
    (if (= room 0) {:first 1 :room 0 :total 0 :lines []}
        (let [layout (misa.transcript.layout db context)
              selected (and misa.selection misa.selection.state
                            (misa.selection.state db))
              key (selection-key selected)
              bottom (math.max 1 (+ (- layout.total room) 1))]
          (var first (math.max 1
                               (math.min (viewport-top db.messages layout
                                                       bottom)
                                         bottom)))
          ;; Reveal the focused selection when the scroll position is stale.
          (when (and selected
                     (not (same-selection? db.messages.scroll_selection key))
                     layout.selected_row)
            (let [index layout.selected_row]
              (set first (math.max 1 (math.min first index)))
              (when (>= index (+ first room))
                (set first (+ (- index room) 1)))))
          {: first
           : room
           :total layout.total
           :selection key
           : layout
           :anchor (anchor-at layout first)
           :lines (misa.transcript.rows db layout first room)}))))

(fn scroll [viewport delta]
  "Calculate the scroll transaction from the current viewport geometry."
  (if (or (not viewport) (= viewport.room 0))
      {:fx [{:type :terminal/read}]}
      (let [bottom (math.max 1 (+ (- viewport.total viewport.room) 1))
            first (math.max 1 (math.min bottom (- viewport.first delta)))]
        {:patch {:messages {:top (if (< first bottom)
                                     first
                                     misa.delete)
                            :anchor (if (< first bottom)
                                        (misa.replace (anchor-at viewport.layout
                                                                 first))
                                        misa.delete)
                            :scroll_selection (misa.replace viewport.selection)
                            :scroll (math.max 0 (- bottom first))}}
         :fx [{:type :terminal/read}]})))

(fn handle-scroll [db event cofx]
  "Read viewport geometry and calculate the requested scroll transaction."
  (let [terminal (or (and cofx cofx.terminal) {:lines 0 :columns 80})
        viewport (if (and misa.ui misa.ui.regions)
                     (accumulate [found nil _ region (ipairs (misa.ui.regions db
                                                                              terminal))]
                       (or found
                           (when (= region.id :transcript)
                             region.viewport)))
                     (viewport db
                               {:columns terminal.columns
                                :images terminal.images
                                :interactive true}
                               terminal.lines))]
    (scroll viewport event.delta)))

{: anchor-at : anchored-row : handle-scroll : scroll : viewport}
