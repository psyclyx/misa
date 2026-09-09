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

(fn anchor-at [lines index]
  (let [anchor (line-anchor (. lines index))]
    (when anchor
      (do
        (var finished? false)
        (for [previous (- index 1) 1 -1 &until finished?]
          (when (not (same-source? anchor (line-anchor (. lines previous))))
            (set finished? true))
          (when (not finished?) (set anchor.offset (+ anchor.offset 1))))))
    anchor))

(fn line-anchors [lines]
  (var (previous offset) (values nil 0))
  (icollect [_ line (ipairs lines)]
    (let [anchor (line-anchor line)]
      (set offset (if (same-source? previous anchor) (+ offset 1) 0))
      (set anchor.offset offset)
      (set previous anchor)
      previous)))

(fn anchored-row [anchors anchor fallback]
  (var (found distance offset-distance) (values nil math.huge math.huge))
  (var (exact exact-offset) (values nil math.huge))
  (when anchor
    (each [index candidate (ipairs anchors)]
      (when (and (= candidate.id anchor.id)
                 (or (not anchor.part) (= candidate.part anchor.part)))
        (let [delta (if (and candidate.last (<= candidate.source anchor.source)
                             (< anchor.source candidate.last))
                        0
                        (math.abs (- candidate.source anchor.source)))
              offset-delta (math.abs (- candidate.offset anchor.offset))]
          ;; After displacement, prefer the original source start if it still
          ;; exists. A containing range is only a fallback for a rewrapped row.
          (when (and (= candidate.source anchor.source)
                     (< offset-delta exact-offset))
            (set (exact exact-offset) (values index offset-delta)))
          (when (or (< delta distance)
                    (and (= delta distance) (< offset-delta offset-distance)))
            (set (found distance offset-distance)
                 (values index delta offset-delta)))))))
  (or exact found fallback))

(fn same-anchor? [a b]
  (and a b (= a.id b.id) (= a.part b.part) (= a.source b.source)
       (= a.offset b.offset)))

(fn viewport-top [messages lines bottom]
  (if (not messages.top) bottom
      ;; Scrolling owns a physical row. Only relocate its content when layout
      ;; changes have actually displaced that row; ordinary renders must not
      ;; reinterpret an explicit scroll as a request to find the block again.
      (or (not messages.anchor)
          (same-anchor? (anchor-at lines messages.top) messages.anchor))
      messages.top (anchored-row (line-anchors lines) messages.anchor
                                messages.top)))

(fn viewport [db context available-lines]
  "Select visible transcript rows and their source anchor."
  (let [room (math.max 0 (math.floor (or available-lines 0)))]
    (if (= room 0) {:first 1 :room 0 :total 0 :lines []}
        (let [lines (misa.transcript.project db context)
              selected (and misa.selection misa.selection.state
                            (misa.selection.state db))
              key (selection-key selected)
              bottom (math.max 1 (+ (- (length lines) room) 1))]
          (var first (math.max 1
                               (math.min (viewport-top db.messages lines bottom)
                                         bottom)))
          (when (and selected
                     (not (same-selection? db.messages.scroll_selection key)))
            (do
              (var finished? false)
              (each [index line (ipairs lines) &until finished?]
                (when (and (or line.selected line.selection_anchor)
                           (or (not line.selection_id)
                               (= line.selection_id selected.id)))
                  (set first (math.max 1 (math.min first index)))
                  (when (>= index (+ first room))
                    (set first (+ (- index room) 1)))
                  (set finished? true)))))
          {: first
           : room
           :total (length lines)
           :selection key
           :layout lines
           :anchor (anchor-at lines first)
           :lines (icollect [index (ipairs lines)
                             &until (>= index (+ first room))]
                    (when (>= index first) (. lines index)))}))))

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

{: handle-scroll : viewport : scroll}
