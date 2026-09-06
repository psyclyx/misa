;; Single geometry projection for every choice overlay. Input handling and the

;; component consume the same rows and positional targets from this projection.

(fn clamp [value low high] (math.max low (math.min high value)))

(fn preview-lines [preview]
  (if (= preview nil) [] (= (type preview) :string) [preview]
      (let [result []]
        (when (= (type preview) :table)
          (when preview.title
            (table.insert result (tostring preview.title)))
          (if (= (type preview.lines) :table)
              (each [_ line (ipairs preview.lines)]
                (table.insert result (tostring line)))
              (let [body []]
                (each [key value (pairs preview)]
                  (when (and (not= key :title) (not= key :lines))
                    (table.insert body
                                  (.. (tostring key) ": " (tostring value)))))
                (table.sort body)
                (each [_ line (ipairs body)] (table.insert result line)))))
        result)))

{:setup (fn [context]
          (local setup-fx [])
          (assert (and misa.layout misa.choice_rows)
                  "choice_layout requires layout and choices")
          (var configured (or (and (and (= (type context.config) :table)
                                        (= (type context.config.choices) :table))
                                   context.config.choices.overlay)
                              nil))
          (set configured (or (and (= (type configured) :table) configured) {}))
          (each [_ name (ipairs [:preferred_width
                                 :min_width
                                 :max_width
                                 :preferred_height
                                 :min_height
                                 :max_height
                                 :panel_min_width])]
            (local value (. configured name))
            (assert (or (= value nil)
                        (and (and (= (type value) :number) (>= value 1))
                             (= (% value 1) 0)))
                    (.. "choice overlay " name " must be a positive integer")))
          (assert (<= (or configured.min_width 28)
                      (or configured.max_width math.huge))
                  "choice overlay min_width exceeds max_width")
          (assert (<= (or configured.min_height 4)
                      (or configured.max_height 18))
                  "choice overlay min_height exceeds max_height")
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_row_lines
                         :value (fn [row width]
                                  (local style
                                         (or (and row.selected
                                                  :choice.row.selected)
                                             (or (and row.active
                                                      :choice.row.active)
                                                 :choice.row)))
                                  (local spans
                                         [{: style
                                           :text (.. (or row.marker " ") " ")}])
                                  (when row.hotkey
                                    (each [_ part (ipairs (or (and misa.render_keybinding
                                                                   (misa.render_keybinding row.hotkey))
                                                              [{:style :keybinding
                                                                :text row.hotkey}]))]
                                      (tset spans (+ (length spans) 1)
                                            {:style (or (and (or row.selected
                                                                 row.active)
                                                             style)
                                                        part.style)
                                             :text part.text}))
                                    (tset spans (+ (length spans) 1)
                                          {: style :text " "}))
                                  (tset spans (+ (length spans) 1)
                                        {: style :text row.label})
                                  (when (and row.description
                                             (not= row.description ""))
                                    (tset spans (+ (length spans) 1)
                                          {:style (or (and (or row.selected
                                                               row.active)
                                                           style)
                                                      :choice.hint)
                                           :text (.. " — " row.description)}))
                                  (local lines
                                         (misa.layout.wrap_spans [{: spans}]
                                                                 width))
                                  (each [_ line (ipairs lines)]
                                    (var used 0)
                                    (each [_ part (ipairs line.spans)]
                                      (set used
                                           (+ used
                                              (misa.layout.width part.text))))
                                    (when (< used width)
                                      (tset line.spans
                                            (+ (length line.spans) 1)
                                            {: style
                                             :text (string.rep " "
                                                               (- width used))}))
                                    (each [_ part (ipairs line.spans)]
                                      (set part.action row.action)))
                                  lines)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_viewport
                         :value (fn [panel models width height bank compact]
                                  (local row-limit (math.max 1 (- height 1)))

                                  (fn window [start]
                                    (var (rows lines targets section)
                                         (values {} {} {}
                                                 (or (and (and compact
                                                               (. models start))
                                                          (. models start
                                                             :section))
                                                     nil)))
                                    (var indices {})
                                    (for [index start (math.min (length panel.items)
                                                                (+ start
                                                                   row-limit -1))]
                                      (tset indices (+ (length indices) 1)
                                            index))
                                    ;; In a compact grouped list, spend the scarce rows on both groups.
                                    ;; Keyboard navigation still reaches every item; retain a focused item
                                    ;; from the leading group when it advances beyond the first entry.
                                    (when (and (and compact section)
                                               (>= height 4))
                                      (var boundary nil)
                                      (for [index (+ start 1) (length models)
                                            &until boundary]
                                        (when (not= (. models index :section)
                                                    section)
                                          (set boundary index)))
                                      (when (and boundary
                                                 (> boundary (+ start 1)))
                                        (set indices
                                             [(math.max start
                                                        (math.min panel.highlight
                                                                  (- boundary 1)))])
                                        (for [index boundary (math.min (length panel.items)
                                                                       (+ boundary
                                                                          row-limit
                                                                          -2))]
                                          (tset indices (+ (length indices) 1)
                                                index))))
                                    (var full false)
                                    (each [_ index (ipairs indices) &until full]
                                      (local row (. models index))
                                      (local shortcut
                                             (when (< (length rows) 9)
                                               (.. :option_ bank "_"
                                                   (+ (length rows) 1))))
                                      (set row.hotkey
                                           (and shortcut
                                                (misa.choice_hint shortcut)))
                                      (set row.action
                                           (and shortcut
                                                (.. :choices. shortcut)))
                                      (local wrapped
                                             (misa.choice_row_lines row width))
                                      (local heading
                                             (or (and (and row.section
                                                           (not= row.section
                                                                 section))
                                                      row.section)
                                                 nil))
                                      (local capacity
                                             (- (- (math.max 0 (- height 1))
                                                   (length lines))
                                                (or (and heading 1) 0)))
                                      (set full
                                           (and (> (length wrapped) capacity)
                                                (or (> (length rows) 0)
                                                    (< capacity 1))))
                                      (when (not full)
                                        (when (> (length wrapped) capacity)
                                          ;; A single tall choice must remain reachable even in a small dock.
                                          ;; Keep its measured visible lines and mark omitted content explicitly.
                                          (while (> (length wrapped) capacity)
                                            (table.remove wrapped))
                                          (local last
                                                 (. wrapped (length wrapped)))
                                          (local text {})
                                          (each [_ part (ipairs last.spans)]
                                            (tset text (+ (length text) 1)
                                                  part.text))
                                          (set last.spans
                                               [{:action row.action
                                                 :style (. last.spans 1 :style)
                                                 :text (.. (misa.layout.clip (table.concat text)
                                                                             (math.max 0
                                                                                       (- width
                                                                                          1)))
                                                           "…")}]))
                                        (when heading
                                          (tset lines (+ (length lines) 1)
                                                {:spans [{:style :choice.view
                                                          :text heading}]}))
                                        (set section row.section)
                                        (set row.source_index index)
                                        (tset rows (+ (length rows) 1) row)
                                        (when shortcut
                                          (tset targets shortcut
                                                (. panel.items index)))
                                        (each [_ line (ipairs wrapped)]
                                          (tset lines (+ (length lines) 1) line))))
                                    (values rows lines targets))

                                  (var start
                                       (misa.choice_first_index panel row-limit))
                                  (var (rows lines targets) (window start))
                                  ;; Fit by physical lines, keeping the focused choice inside the viewport.

                                  (fn focused []
                                    (var found false)
                                    (each [_ row (ipairs rows) &until found]
                                      (set found
                                           (= row.source_index panel.highlight)))
                                    found)

                                  (while (and (< start panel.highlight)
                                              (not (focused)))
                                    (set start (+ start 1))
                                    (set (rows lines targets) (window start)))
                                  (local last
                                         (or (and (. rows (length rows))
                                                  (. rows (length rows)
                                                     :source_index))
                                             (- start 1)))
                                  (var overflow
                                       (or (and (and (> height 0)
                                                     (or (or (> start 1)
                                                             (< last
                                                                (length panel.items)))
                                                         (> (+ (- last start) 1)
                                                            (length rows))))
                                                (.. (or (and compact
                                                             (tostring (length rows)))
                                                        (or (and (> (length rows)
                                                                    0)
                                                                 (.. start
                                                                     "–" last))
                                                            :0))
                                                    " / " (length panel.items)
                                                    "  ↑↓ more"))
                                           nil))
                                  (when overflow
                                    (local shown {})
                                    (each [_ row (ipairs rows)]
                                      (when row.section
                                        (tset shown row.section true)))
                                    (for [index (+ last 1) (length models)]
                                      (local section (. models index :section))
                                      (when (and section
                                                 (not (. shown section)))
                                        (set overflow
                                             (.. overflow " · " section))
                                        (tset shown section true))))
                                  {: lines : overflow : rows : targets})})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_picker_layout
                         :value (fn [session db terminal]
                                  (misa.choice_refresh session db)
                                  (local compact (= terminal.compact true))
                                  (local screen-width
                                         (math.max 1
                                                   (math.floor (or terminal.columns
                                                                   1))))
                                  (var available terminal.available_lines)
                                  (when (= available nil)
                                    (set available
                                         (or (or (and misa.picker_available_lines
                                                      (misa.picker_available_lines db
                                                                                   terminal))
                                                 terminal.lines)
                                             0)))
                                  (set available
                                       (math.max 0 (math.floor available)))
                                  (var min-width
                                       (math.min screen-width
                                                 (or configured.min_width 28)))
                                  (var max-width
                                       (math.min screen-width
                                                 (or configured.max_width
                                                     screen-width)))
                                  (local padding
                                         (math.min (math.floor (/ (- screen-width
                                                                     1)
                                                                  2))
                                                   (or configured.padding 2)))
                                  (set max-width
                                       (math.max 1
                                                 (math.min max-width
                                                           (- screen-width
                                                              (* padding 2)))))
                                  (set min-width (math.min min-width max-width))
                                  (local width
                                         (clamp (or configured.preferred_width
                                                    max-width)
                                                min-width max-width))
                                  (local x padding)
                                  (local min-height
                                         (math.min available
                                                   (or configured.min_height 4)))
                                  (local max-height
                                         (math.min available
                                                   (or configured.max_height 18)))
                                  (local height
                                         (or (and compact available)
                                             (clamp (or configured.preferred_height
                                                        14)
                                                    min-height max-height)))
                                  (local widths
                                         (misa.layout.columns width
                                                              (or configured.panel_min_width
                                                                  28)
                                                              (math.min (length session.panels)
                                                                        3)
                                                              2))
                                  (local highlighted
                                         (and (. session.panels 1)
                                              (. session.panels 1 :items
                                                 (. session.panels 1 :highlight))))
                                  (local info
                                         (and highlighted highlighted.preview))
                                  (local preview
                                         (or (and (and (and compact
                                                            (= (type info)
                                                               :table))
                                                       info.summary)
                                                  [(tostring info.summary)])
                                             (preview-lines info)))
                                  (local preview-height
                                         (math.min (length preview)
                                                   (or (and compact
                                                            (or (and (>= height
                                                                         2)
                                                                     1)
                                                                0))
                                                       (math.max 0
                                                                 (math.floor (/ height
                                                                                3))))))
                                  (var hint-actions
                                       [[:previous :previous]
                                        [:next :next]
                                        [:accept :accept]
                                        [:cancel :cancel]
                                        [:cycle "cycle views"]
                                        [:replace_view :views]])
                                  (when session.preference_scope
                                    (tset hint-actions
                                          (+ (length hint-actions) 1)
                                          [:favorite :favorite]))
                                  (when compact
                                    (set hint-actions
                                         (or (and (>= height 8)
                                                  [[:open_overlay :expand]
                                                   [:cycle :views]
                                                   [:favorite :favorite]])
                                             {})))
                                  (local hints {})
                                  (each [_ entry (ipairs hint-actions)]
                                    (local key (misa.choice_hint (. entry 1)))
                                    (when key
                                      (tset hints (+ (length hints) 1)
                                            {:action (.. :choices. (. entry 1))
                                             : key
                                             :label (. entry 2)
                                             :tokens (or (and misa.keybinding_tokens
                                                              (misa.keybinding_tokens key))
                                                         nil)})))
                                  (local input-height (or (and compact 0) 1))
                                  (local hint-height
                                         (or (and (> (length hints) 0) 1) 0))
                                  (local fixed
                                         (+ input-height preview-height
                                            hint-height))
                                  (local panel-budget
                                         (math.max 0 (- height fixed)))
                                  (local all (misa.choice_rows session db))
                                  (local (columns targets) (values {} {}))
                                  (var column-x 0)
                                  (each [bank panel-width (ipairs widths)]
                                    (local panel (. session.panels bank))
                                    (local viewport
                                           (misa.choice_viewport panel
                                                                 (. all bank
                                                                    :rows)
                                                                 panel-width
                                                                 (math.max 0
                                                                           (- panel-budget
                                                                              1))
                                                                 bank compact))
                                    (each [action item (pairs viewport.targets)]
                                      (tset targets action item))
                                    (tset columns bank
                                          {:active (= bank 1)
                                           :id panel.id
                                           :lines viewport.lines
                                           :overflow viewport.overflow
                                           :rows viewport.rows
                                           :title (or (and (and compact
                                                                (. viewport.rows
                                                                   1))
                                                           (. viewport.rows 1
                                                              :section))
                                                      panel.title)
                                           :width panel-width
                                           :x column-x
                                           :y (+ input-height preview-height)})
                                    (set column-x (+ column-x panel-width 2)))
                                  (local query
                                         (.. (or session.input_prefix "")
                                             session.query))
                                  {:available_lines available
                                   : columns
                                   : height
                                   :hint_height hint-height
                                   :hint_y (+ input-height preview-height
                                              panel-budget)
                                   : hints
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
                                             :lines preview
                                             : width
                                             : x
                                             :y input-height}
                                   : targets
                                   : width
                                   : x
                                   :y 0})})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_completion_layout
                         :value (fn [session db columns lines]
                                  (misa.choice_picker_layout session db
                                                             {:available_lines lines
                                                              : columns
                                                              :compact true}))})
          {:fx setup-fx})}
