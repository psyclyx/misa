(local definitions (require :misa.definitions))

;; Root composition only. Feature state and viewport extraction remain owned by
;; editor, messages, picker layers, and status.

(fn append [target source limit]
  (each [_ line (ipairs (or source {}))]
    (when (or (not limit) (< (length target) limit))
      (tset target (+ (length target) 1) line))))

(fn slice [lines first count]
  (let [result {}]
    (for [index (math.max 1 first) (math.min (length lines)
                                             (- (+ first count) 1))]
      (tset result (+ (length result) 1) (. lines index)))
    result))

(fn bound-frame [lines columns cursor]
  "Clip a composed frame to the available terminal dimensions."
  (let []
    ;; Clip the whole semantic line before projecting its byte boundary through spans: graphemes
    ;; may cross style/link boundaries and must never be partially retained.
    (local budget (math.max 0 (math.floor (or (tonumber columns) 1))))
    (local result {})
    (each [_ line (ipairs lines)]
      (var full "")
      (each [_ source (ipairs (or line.spans {}))]
        (set full (.. full (or source.text ""))))
      (local (_ visible-end) (misa.layout.clip full budget))
      (var (spans offset) (values {} 0))
      (each [_ source (ipairs (or line.spans {}))]
        (local text (or source.text ""))
        (local count
               (math.min (length text) (math.max 0 (- visible-end offset))))
        (when (or (> count 0) (and (= (length text) 0) (<= offset visible-end)))
          (local item {})
          (each [key value (pairs source)] (tset item key value))
          (set item.text (text:sub 1 count))
          (when (< count (length text)) (set item.animation nil))
          (tset spans (+ (length spans) 1) item))
        (set offset (+ offset (length text))))
      (local copy {})
      (each [key value (pairs line)] (tset copy key value))
      (set copy.spans spans)
      (tset result (+ (length result) 1) copy))
    (var bounded-cursor nil)
    (each [index line (ipairs result)]
      (when line.image
        (var count 1)
        (while (and (. result (+ index count))
                    (= (. result (+ index count) :image_row) line.image.id))
          (set count (+ count 1)))
        (local rectangle {})
        (each [key value (pairs line.image)] (tset rectangle key value))
        (set rectangle.rows (math.min rectangle.rows count))
        (set line.image rectangle)))
    (when cursor
      (local source (. lines cursor.row))
      (var full "")
      (each [_ item (ipairs (or (and source source.spans) {}))]
        (set full (.. full (or item.text ""))))
      (local (_ visible-end) (misa.layout.clip full budget))
      (set bounded-cursor {:byte (math.min (misa.layout.boundary-at-or-before full
                                                                              cursor.byte)
                                           visible-end)
                           :row cursor.row
                           :shape cursor.shape}))
    {:cursor bounded-cursor :lines result}))

;; Shared layout policy for rendering and positional-key resolution.

(local policy {:completions {:height_fraction 2}
               :dock {:transcript_reserve 1}
               :editor {:height_fraction 2}
               :header {:maximum_lines 2 :minimum_height 4}
               :status {:maximum_lines 1 :minimum_height 2}})

(fn chrome [db terminal]
  (let [height (math.max 0 terminal.lines)
        header (. (misa.components.render db :root.header {}) :lines)
        status (or (and misa.status misa.status.model
                        (misa.status.model db {:columns terminal.columns}))
                   {})
        counts {}]
    (each [name lines (pairs {: header : status})]
      (tset counts name (or (and (>= height (. policy name :minimum_height))
                                 (math.min (. policy name :maximum_lines)
                                           (length lines)))
                            0)))
    {:available (math.max 0 (- (- height counts.header) counts.status))
     : counts
     : header
     : height
     : status}))

(fn input-budgets [frame input-count dock-count]
  "Allocate editor, dock, and completion rows within an available frame."
  (let [editor (math.min input-count
                         (math.max 1
                                   (math.floor (/ frame.height
                                                  policy.editor.height_fraction)))
                         (math.max 1 frame.available))]
    (var remaining (math.max 0 (- frame.available editor)))
    (local dock
           (math.min dock-count
                     (math.max 0 (- remaining policy.dock.transcript_reserve))))
    (set remaining (- remaining dock))
    {:completions (math.max 0
                            (math.floor (/ remaining
                                           policy.completions.height_fraction)))
     : dock
     : editor
     : remaining}))

;; Screens are ordered regions. Cursor coordinates belong to their region;
;; this fold is the sole owner of screen offsets.

(fn compose [regions terminal]
  (var (lines cursor) (values {} nil))
  (each [_ region (ipairs regions)]
    (local offset (length lines))
    (append lines region.lines
            (+ offset (or region.count (length (or region.lines {})))))
    (when region.cursor
      (set cursor
           {:byte region.cursor.byte
            :shape region.cursor.shape
            :row (math.max 1
                           (math.min terminal.lines
                                     (+ offset region.cursor.row)))})))
  (bound-frame lines terminal.columns cursor))

(fn regions [db terminal projecting]
  "Compose independently owned presentation regions for a terminal."
  (let [frame (chrome db terminal)
        header {:count frame.counts.header :lines frame.header}
        status {:count frame.counts.status :lines frame.status}
        dock []]
    (var overlay nil)
    (var exclusive nil)
    (var disabled false)
    (each [_ layer (ipairs (misa.ui.layers db
                                           {:available_lines frame.available
                                            : projecting
                                            : terminal}))]
      (when (not exclusive)
        (if layer.exclusive (set exclusive layer)
            (and layer.overlay
                 (or (not overlay)
                     (> (or layer.priority 0) (or overlay.priority 0)))) (set overlay
                                                                                          layer)
            (= layer.dock :input) (do
                                    (append dock layer.lines)
                                    (set disabled
                                         (or disabled layer.input_disabled))))))

    (fn transcript [room]
      (local context {:columns terminal.columns
                      :images terminal.images
                      :interactive true})
      (local viewport
             (and misa.transcript misa.transcript.viewport
                  (misa.transcript.viewport db context room)))
      {:id :transcript
       :count room
       : viewport
       :lines (if viewport viewport.lines
                  misa.transcript.window (misa.transcript.window db context
                                                                 room)
                  [])})

    (if exclusive
        [header
         {:count (math.max 0 (- frame.height frame.counts.header))
          :cursor exclusive.cursor
          :lines exclusive.lines}]
        overlay
        (let [count (math.min (length overlay.lines) frame.available)]
          [header
           (transcript (- frame.available count))
           {: count :cursor overlay.cursor :lines overlay.lines}
           status])
        (let [editor (if (and misa.editor misa.editor.layout)
                         (misa.editor.layout db
                                             {: terminal
                                              :layout {:height frame.height
                                                       :available frame.available
                                                       :dock_count (length dock)}})
                         {:busy true :byte 0 :completions [] :input [] :row 1})
              budgets (input-budgets frame (length editor.input) (length dock))
              first (math.max 1
                              (math.min (- editor.row
                                           (math.floor (/ budgets.editor 2)))
                                        (+ (- (length editor.input)
                                              budgets.editor)
                                           1)))
              completion-count (math.min (length editor.completions)
                                         budgets.completions)]
          [header
           (transcript (- budgets.remaining completion-count))
           {:count budgets.dock :lines dock}
           {:cursor (when (not disabled)
                      {:byte editor.byte
                       :shape editor.shape
                       :row (+ (- editor.row first) 1)})
            :lines (slice editor.input first budgets.editor)}
           {:count completion-count :lines editor.completions}
           status]))))

(fn []
  "Build the declarations for ui."
  (local declarations [{:catalog :services :id :ui.regions :value regions}
                       {:catalog :services
                        :id :ui.input-budgets
                        :value input-budgets}])
  (table.insert declarations
                {:catalog :services
                 :id :ui.overlay-room
                 :value (fn [db terminal]
                          "Return the number of rows available to an overlay."
                          (. (chrome db terminal) :available))})
  (table.insert declarations {:catalog :services
                              :id :ui.bound-frame
                              :value bound-frame})
  (table.insert declarations
                {:catalog :services
                 :id :ui.picker-room
                 :value (fn [db terminal]
                          "Return the number of rows available to a picker."
                          (let [frame (chrome db terminal)]
                            (math.max 0 (- frame.height frame.counts.header))))})
  (table.insert declarations
                {:catalog :services
                 :id :ui.completion-room
                 :value (fn [db terminal input-count]
                          "Return the number of rows available to inline completions."
                          (let [frame (chrome db terminal)]
                            (if (= frame.height 0)
                                0
                                (do
                                  (var dock-count 0)
                                  (each [_ layer (ipairs (misa.ui.layers db
                                                                         {:available_lines frame.available
                                                                          : terminal}))]
                                    (when (= layer.dock :input)
                                      (set dock-count
                                           (+ dock-count
                                              (length (or layer.lines []))))))
                                  (. (input-budgets frame input-count
                                                    dock-count)
                                     :completions)))))})
  (table.insert declarations
                {:catalog :views
                 :id :main
                 :value (fn [db cofx]
                          (if (<= cofx.terminal.lines 0) {:lines []}
                              (compose (regions db cofx.terminal
                                                cofx.projecting)
                                       cofx.terminal)))})
  (definitions :ui declarations {}))
