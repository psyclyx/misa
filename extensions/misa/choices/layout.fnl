;; Single geometry projection for every choice overlay. Input handling and the
;; component consume the same rows and positional targets from this projection.

(fn clamp [value low high] (math.max low (math.min high value)))

(fn choices-row-lines [row width]
  "Render a choice row within the supplied width."
  (let [style (or (and row.selected :choice.row.selected)
                  (and row.active :choice.row.active) :choice.row)
        spans [{: style :text (.. (or row.marker " ") " ")}]]
    (when row.hotkey
      (var entered "")
      (var index 0)
      (each [key (row.hotkey:gmatch "%S+")]
        (set index (+ index 1))
        (set entered (if (= index 1) key (.. entered " " key)))
        (let [progress (and row.combo
                            (= (row.hotkey:sub 1 (+ (length row.combo) 1))
                               (.. row.combo " "))
                            (= (row.combo:sub 1 (length entered)) entered))]
          (when (> index 1)
            (table.insert spans {: style :text " " :sequence_progress progress}))
          (each [_ part (ipairs (misa.keybindings.render key))]
            (table.insert spans
                          {:style (if progress
                                      :choice.row.active
                                      (or (and (or row.selected row.active)
                                               style)
                                          part.style))
                           :sequence_progress progress
                           :text part.text}))))
      (tset spans (+ (length spans) 1) {: style :text " "}))
    (tset spans (+ (length spans) 1) {: style :text row.label})
    (when (and row.description (not= row.description ""))
      (tset spans (+ (length spans) 1)
            {:style (or (and (or row.selected row.active) style) :choice.hint)
             :text (.. " — " row.description)}))
    (let [lines (misa.layout.wrap-spans [{: spans}] width)]
      (each [_ line (ipairs lines)]
        (var used 0)
        (each [_ part (ipairs line.spans)]
          (set used (+ used (misa.layout.width part.text))))
        (when (< used width)
          (tset line.spans (+ (length line.spans) 1)
                {: style :text (string.rep " " (- width used))}))
        (each [_ part (ipairs line.spans)]
          (set part.action row.action)))
      lines)))

(fn choices-viewport [panel models width height bank compact combo]
  "Calculate the visible interval for a focused choice panel."
  (var content-height height)
  (let [row-limit (math.max 1 height)]
    (fn window [start]
      (var (rows lines targets section)
           (values {} {} {} (or (and compact (. models start)
                                     (. models start :section))
                                nil)))
      (let [indices {}]
        (for [index start (math.min (length panel.items) (+ start row-limit -1))]
          (tset indices (+ (length indices) 1) index))
        (var full false)
        (each [_ index (ipairs indices) &until full]
          (let [row (. models index)
                shortcut (.. :option_ bank "_" (+ (length rows) 1))]
            (set row.combo combo)
            (set row.hotkey (and shortcut (misa.choices.hint shortcut)))
            (set row.action (and shortcut (.. :choices. shortcut)))
            (let [wrapped (misa.choices.row-lines row width)
                  heading (or (and row.section (not= row.section section)
                                   row.section)
                              nil)
                  capacity (- (- (math.max 0 content-height) (length lines))
                              (or (and heading 1) 0))]
              (set full
                   (and (> (length wrapped) capacity)
                        (or (> (length rows) 0) (< capacity 1))))
              (when (not full)
                (when (> (length wrapped) capacity)
                  ;; A single tall choice must remain reachable even in a small dock.
                  ;; Keep its measured visible lines and mark omitted content explicitly.
                  (while (> (length wrapped) capacity)
                    (table.remove wrapped))
                  (let [last (. wrapped (length wrapped))
                        text {}]
                    (each [_ part (ipairs last.spans)]
                      (tset text (+ (length text) 1) part.text))
                    (set last.spans
                         [{:action row.action
                           :style (. last.spans 1 :style)
                           :text (.. (misa.layout.clip (table.concat text)
                                                       (math.max 0 (- width 1)))
                                     "…")}])))
                (when heading
                  (tset lines (+ (length lines) 1)
                        {:spans [{:style :choice.view :text heading}]}))
                (set section row.section)
                (set row.source_index index)
                (tset rows (+ (length rows) 1) row)
                (when shortcut
                  (tset targets shortcut (. panel.items index)))
                (each [_ line (ipairs wrapped)]
                  (tset lines (+ (length lines) 1) line))))))
        (values rows lines targets)))

    (var start (misa.choices.first-index panel row-limit))
    (var (rows lines targets) (window start))
    ;; Fit by physical lines, keeping the focused choice inside the viewport.

    (fn focused []
      (var found false)
      (each [_ row (ipairs rows) &until found]
        (set found (= row.source_index panel.highlight)))
      found)

    (while (and (< start panel.highlight) (not (focused)))
      (set start (+ start 1))
      (set (rows lines targets) (window start)))
    ;; Charge for overflow only when there are hidden choices.
    (when (and (> height 1)
               (or (> start 1)
                   (< (or (and (. rows (length rows))
                               (. rows (length rows) :source_index))
                          0) (length panel.items))))
      (set content-height (- height 1))
      (set (rows lines targets) (window start))
      (while (and (< start panel.highlight) (not (focused)))
        (set start (+ start 1))
        (set (rows lines targets) (window start))))
    (let [last (or (and (. rows (length rows))
                        (. rows (length rows) :source_index))
                   (- start 1))
          overflow (or (and (> height 0)
                            (or (> start 1) (< last (length panel.items))
                                (> (+ (- last start) 1) (length rows)))
                            (.. (or (and compact (tostring (length rows)))
                                    (and (> (length rows) 0)
                                         (.. start "–" last))
                                    :0) " / "
                                (length panel.items) "  ↑↓ more"))
                       nil)]
      {: lines : overflow : rows : targets})))

(fn choices-completion-layout [session db columns lines]
  "Lay out inline completions within the available dimensions."
  (misa.choices.picker-layout session db
                              {:available_lines lines : columns :compact true}))

(fn overlay-options [config]
  "Validate and return choice overlay dimensions."
  (let [value (. (or config.choices {}) :overlay)
        configured (if (= (type value) :table) value {})]
    (each [_ name (ipairs [:preferred_width
                           :min_width
                           :max_width
                           :preferred_height
                           :min_height
                           :max_height
                           :panel_min_width])]
      (let [value (. configured name)]
        (assert (or (= value nil)
                    (and (= (type value) :number) (>= value 1)
                         (= (% value 1) 0)))
                (.. "choice overlay " name " must be a positive integer"))))
    (assert (<= (or configured.min_width 28)
                (or configured.max_width math.huge))
            "choice overlay min_width exceeds max_width")
    (assert (<= (or configured.min_height 4) (or configured.max_height 18))
            "choice overlay min_height exceeds max_height")
    configured))

(fn choices-picker-layout [configured previous db terminal]
  "Lay out choice panels within the available terminal dimensions."
  (let [session (misa.choices.refresh previous db)
        compact (= terminal.compact true)
        screen-width (math.max 1 (math.floor (or terminal.columns 1)))]
    (var available terminal.available_lines)
    (when (= available nil)
      (set available (or (and misa.ui misa.ui.picker-room
                              (misa.ui.picker-room db terminal))
                         terminal.lines 0)))
    (set available (math.max 0 (math.floor available)))
    (var min-width (math.min screen-width (or configured.min_width 28)))
    (var max-width
         (math.min screen-width (or configured.max_width screen-width)))
    (let [padding (math.min (math.floor (/ (- screen-width 1) 2))
                            (or configured.padding 2))]
      (set max-width
           (math.max 1 (math.min max-width (- screen-width (* padding 2)))))
      (set min-width (math.min min-width max-width))
      (let [width (clamp (or configured.preferred_width max-width) min-width
                         max-width)
            x padding
            min-height (math.min available (or configured.min_height 4))
            max-height (math.min available (or configured.max_height 18))
            height (or (and compact available)
                       (clamp (or configured.preferred_height 14) min-height
                              max-height))
            widths (misa.layout.columns width
                                        (or configured.panel_min_width 28)
                                        (math.min (length session.panels) 3) 2)
            highlighted (and (. session.panels 1)
                             (. session.panels 1 :items
                                (. session.panels 1 :highlight)))
            info (and highlighted highlighted.preview)
            preview (misa.choices.preview info {:columns width : compact})
            preview-height (math.min (length preview)
                                     (or (and compact
                                              (or (and (>= height 2) 1) 0))
                                         (math.max 0 (math.floor (/ height 3)))))]
        (var hint-actions [[:complete :complete]
                           [:previous :previous]
                           [:next :next]
                           [:accept :accept]
                           [:cancel :cancel]
                           [:cycle "cycle views"]
                           [:replace_view :views]])
        (when session.preference_scope
          (tset hint-actions (+ (length hint-actions) 1) [:favorite :favorite]))
        (when compact
          (set hint-actions (or (and (>= height 8)
                                     [[:complete :complete]
                                      [:cycle :views]
                                      [:favorite :favorite]])
                                {})))
        (let [hints {}]
          (each [_ entry (ipairs hint-actions)]
            (let [key (misa.choices.hint (. entry 1))]
              (when key
                (tset hints (+ (length hints) 1)
                      {:action (.. :choices. (. entry 1))
                       : key
                       :label (. entry 2)
                       :tokens (misa.keybindings.tokens key)}))))
          (let [input-height (or (and compact 0) 1)
                hint-height (or (and (> (length hints) 0) 1) 0)
                fixed (+ input-height preview-height hint-height)
                panel-budget (math.max 0 (- height fixed))
                all (misa.choices.projected-rows session)
                columns {}
                targets {}]
            (var column-x 0)
            (each [bank panel-width (ipairs widths)]
              (let [panel (. session.panels bank)
                    viewport (misa.choices.viewport panel (. all bank :rows)
                                                    panel-width
                                                    (math.max 0
                                                              (- panel-budget 1))
                                                    bank compact session.combo)]
                (each [action item (pairs viewport.targets)]
                  (tset targets action item))
                (tset columns bank
                      {:active (= bank 1)
                       :id panel.id
                       :lines viewport.lines
                       :overflow viewport.overflow
                       :rows viewport.rows
                       :title (or (and compact (. viewport.rows 1)
                                       (. viewport.rows 1 :section))
                                  panel.title)
                       :width panel-width
                       :x column-x
                       :y (+ input-height preview-height)})
                (when (and session.combo (= bank 1))
                  (tset columns bank :title
                        (.. (. columns bank :title) " ["
                            (misa.keybindings.text session.combo) " …]")))
                (set column-x (+ column-x panel-width 2))))
            (let [query (.. (or session.input_prefix "") session.query)]
              {:available_lines available
               : columns
               : height
               :hint_height hint-height
               :hint_y (+ input-height preview-height panel-budget)
               : hints
               :combo session.combo
               :input {:cursor (length query)
                       :hidden compact
                       :text query
                       :title session.title
                       : width
                       : x
                       :y 0}
               :panel_count (length columns)
               :panel_height panel-budget
               :panel_y (+ input-height preview-height)
               :preview {:height preview-height
                         :model info
                         :lines preview
                         : width
                         : x
                         :y input-height}
               : targets
               : width
               : x
               :y 0})))))))

{: choices-completion-layout
 : choices-picker-layout
 : choices-row-lines
 : choices-viewport
 : overlay-options}
