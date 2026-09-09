(local definitions (require :misa.definitions))

;; Generic structural navigation over source documents supplied by features.
;; Selection state contains a frozen snapshot: streaming cannot move the range
;; between a user's navigation keystroke and copy.

(local scrolling-inputs {:wheel_up true
                         :wheel_down true
                         :page_up true
                         :page_down true})

(fn focus [state]
  (let [frame (. state.frames (length state.frames))]
    (values (. frame.nodes frame.index) frame)))

(fn frames-with [frames index frame]
  (icollect [i entry (ipairs frames)] (if (= i index) frame entry)))

(fn move [state index extending]
  (let [depth (length state.frames)
        (_ frame) (focus state)
        next-frame (misa.patch frame
                               {:index (math.max 1
                                                 (math.min (length frame.nodes)
                                                           index))})
        current (. next-frame.nodes next-frame.index)
        document-index (if (= depth 1) next-frame.index
                           (. state.frames 1 :index))
        anchor (if extending
                   state.anchor
                   current)
        anchor-document (if extending state.anchor_document document-index)
        forward (and anchor-document (<= anchor-document document-index))]
    (misa.patch state
                {:frames (misa.replace (frames-with state.frames depth
                                                    next-frame))
                 :anchor (misa.replace anchor)
                 :anchor_document anchor-document
                 :range (misa.replace (when (and extending anchor current)
                                        {:first_document (math.min anchor-document
                                                                   document-index)
                                         :last_document (math.max anchor-document
                                                                  document-index)
                                         :first (if (= anchor-document
                                                       document-index)
                                                    (math.min anchor.first
                                                              current.first)
                                                    forward
                                                    anchor.first
                                                    current.first)
                                         :last (if (= anchor-document
                                                      document-index)
                                                   (math.max anchor.last
                                                             current.last)
                                                   forward
                                                   current.last
                                                   anchor.last)}))})))

(fn selected-at [state index]
  (let [current (focus state)
        focused (. state.frames 1 :index)
        range (or state.range
                  (and current
                       {:first_document focused
                        :last_document focused
                        :first current.first
                        :last current.last}))
        document (. state.documents index)]
    (when (and range document
               (<= range.first_document index range.last_document))
      {:id document.id
       :text document.text
       :source_part document.source_part
       :kind (if (= index focused) current.kind document.kind)
       :first (if (= index range.first_document) range.first 0)
       :last (if (= index range.last_document) range.last
                 (length document.text))})))

(fn descend [state]
  (let [current (focus state)
        document (. state.documents (. state.frames 1 :index))
        children (and current (misa.selection.children document current))]
    (if (and children (> (length children) 0))
        (let [frames (icollect [_ frame (ipairs state.frames)] frame)]
          (table.insert frames {:index 1 :nodes children})
          (move (misa.patch state {:frames (misa.replace frames)}) 1
                state.visual))
        state)))

(fn ascend [state]
  (if (> (length state.frames) 1)
      (let [frames (icollect [i frame (ipairs state.frames)
                              &until (= i (length state.frames))]
                     frame)
            next (misa.patch state {:frames (misa.replace frames)})]
        (move next (. frames (length frames) :index) state.visual))
      state))

(fn rendered-geometry [lines]
  (let [segments []]
    (each [row line (ipairs lines)]
      (var column 0)
      (each [_ span (ipairs (or line.spans []))]
        (when (and (or span.source span.selection_marker)
                   (= (type span.source_start) :number)
                   (= (type span.source_end) :number))
          (table.insert segments
                        {:first span.source_start
                         :last span.source_end
                         : row
                         : column
                         :text span.text}))
        (set column (+ column (misa.layout.width (or span.text ""))))))
    (table.sort segments (fn [a b]
                           (if (= a.first b.first) (< a.last b.last)
                               (< a.first b.first))))
    segments))

(fn geometry-at [segments node]
  (var (low high) (values 1 (length segments)))
  (while (<= low high)
    (let [middle (math.floor (/ (+ low high) 2))]
      (if (<= (. segments middle :last) node.first) (set low (+ middle 1))
          (set high (- middle 1)))))
  (var (first-row last-row first-column) nil)
  (for [index low (length segments)
        &until (>= (. segments index :first) node.last)]
    (let [segment (. segments index)
          column (+ segment.column (if (= (- segment.last segment.first)
                                          (length segment.text))
                                       (misa.layout.width (segment.text:sub 1
                                                                            (math.max 0
                                                                                      (- node.first
                                                                                         segment.first))))
                                       0))]
      (when (or (not first-row) (< segment.row first-row)
                (and (= segment.row first-row) (< column first-column)))
        (set (first-row first-column) (values segment.row column)))
      (set last-row (math.max (or last-row segment.row) segment.row))))
  (values first-row last-row first-column))

(fn directional [state direction geometry]
  (let [current (focus state)
        depth (length state.frames)
        vertical (or (= direction :up) (= direction :down))
        forward (or (= direction :down) (= direction :right))]
    (if (not current) state (= depth 1)
        (if vertical (move state
                           (+ (. state.frames 1 :index) (if forward 1 -1))
                           state.visual) state)
        (let [document (. state.documents (. state.frames 1 :index))
              starts [0]]
          (when (not geometry)
            (each [at (document.text:gmatch "()\n")] (table.insert starts at)))

          (fn row-at [offset]
            (var (low high) (values 1 (length starts)))
            (while (<= low high)
              (let [middle (math.floor (/ (+ low high) 2))]
                (if (<= (. starts middle) offset) (set low (+ middle 1))
                    (set high (- middle 1)))))
            (math.max 1 high))

          (fn column-at [node row]
            (misa.layout.width (document.text:sub (+ (. starts row) 1)
                                                  node.first)))

          (fn bounds [node]
            (if geometry (geometry node)
                (let [row (row-at node.first)]
                  (values row (row-at node.last) (column-at node row)))))

          (let [(row _ column) (bounds current)]
            (if (not row) state
                (let [preferred (if vertical (or state.preferred_column column)
                                    column)]
                  (var (best best-row best-distance) (values nil nil math.huge))

                  (fn visit [node frames]
                    (let [(first-row last-row x) (bounds node)
                          relevant (and first-row
                                        (if vertical
                                            (if forward
                                                (and (> last-row row)
                                                     (or (not best-row)
                                                         (<= first-row best-row)))
                                                (and (< first-row row)
                                                     (or (not best-row)
                                                         (>= last-row best-row))))
                                            (<= first-row row last-row)))]
                      (when relevant
                        (if (= (length frames) depth)
                            (let [valid (if vertical
                                            (if forward (> first-row row)
                                                (< first-row row))
                                            (and (= first-row row)
                                                 (if forward (> x column)
                                                     (< x column))))]
                              (when valid
                                (let [distance (math.abs (- x preferred))]
                                  (when (or (not best)
                                            (and vertical
                                                 (< (math.abs (- first-row row))
                                                    (math.abs (- best-row row))))
                                            (and (= first-row best-row)
                                                 (< distance best-distance)))
                                    (set (best best-row best-distance)
                                         (values frames first-row distance))))))
                            (let [children (misa.selection.children document
                                                                    node)]
                              (for [step 1 (length children)]
                                (let [index (if forward step
                                                (+ (- (length children) step) 1))
                                      path (icollect [_ frame (ipairs frames)]
                                             frame)]
                                  (table.insert path {:nodes children : index})
                                  (visit (. children index) path))))))))

                  (visit document [(. state.frames 1)])
                  (if best
                      (misa.patch (move (misa.patch state
                                                    {:frames (misa.replace best)})
                                        (. best depth :index) state.visual)
                                  {:preferred_column (if vertical preferred
                                                         misa.delete)})
                      (if vertical state
                          (misa.patch state {:preferred_column misa.delete}))))))))))

(local motions {:previous (fn [frame] (- frame.index 1))
                :next (fn [frame] (+ frame.index 1))
                :extend_previous (fn [frame] (- frame.index 1))
                :extend_next (fn [frame] (+ frame.index 1))
                :first (fn [] 1)
                :last (fn [frame] (length frame.nodes))})

(fn selection-geometry [db terminal]
  "Project complete source geometry for the focused frozen document."
  (let [state db.selection
        document (and state (. state.documents (. state.frames 1 :index)))
        provider (and document state.sources
                      (. (misa.catalog :selection-sources)
                         (. state.sources document.id) :layout))]
    (when provider
      {:id document.id
       :text document.text
       :source_part document.source_part
       :segments (rendered-geometry (provider db document terminal))})))

(fn selection-state [db id]
  "Return the current selection state."
  (let [state db.selection]
    (when state
      (let [index (if id (. state.indices id) (. state.frames 1 :index))]
        (when index (selected-at state index))))))

(fn selection-ranges [db]
  "Return the selected source ranges for a document."
  (if (not db.selection) []
      (icollect [index _ (ipairs db.selection.documents)]
        (selected-at db.selection index))))

(fn selection-decorate [db id text lines]
  "Apply selection styling to rendered source spans."
  (let [selected (misa.selection.state db id)]
    (if (or (not selected) (not= selected.id id))
        lines
        (let [highlight (misa.themes.style db :selection)]
          (var (cursor block-start) (values 0 nil))
          (let [decorated []]
            (var anchored false)
            (each [_ original (ipairs lines)]
              (let [line {}]
                (each [key value (pairs original)]
                  (tset line key value))
                (when (and (not= line.source_start nil)
                           (not= line.source_start block-start))
                  (set (cursor block-start)
                       (values line.source_start line.source_start)))
                (let [spans {}
                      whole (and (= selected.first 0)
                                 (= selected.last (length selected.text)))]
                  (each [_ item (ipairs (or line.spans {}))]
                    (var (at parts marked parts-start) (values 0 {} nil 0))

                    (fn flush []
                      (if (= (length parts) 0) nil
                          (let [next {}]
                            (each [key value (pairs item)]
                              (tset next key value))
                            (set next.text (table.concat parts))
                            (when (and item.source_start item.source_end
                                       (= (- item.source_end item.source_start)
                                          (length item.text)))
                              (set next.source_start
                                   (+ item.source_start parts-start))
                              (set next.source_end
                                   (+ next.source_start (length next.text))))
                            (when marked
                              (set next.style {})
                              (each [key value (pairs (or item.style {}))]
                                (tset next.style key value))
                              (each [key value (pairs highlight)]
                                (tset next.style key value))
                              (set line.selected true))
                            (tset spans (+ (length spans) 1) next)
                            (set parts {})
                            nil)))

                    (while (< at (length item.text))
                      (let [after (misa.layout.next-boundary item.text at)
                            piece (item.text:sub (+ at 1) after)]
                        (var active
                             (and item.selection_marker
                                  (or whole
                                      (and line.source_start line.source_end
                                           (<= selected.first line.source_start)
                                           (>= selected.last line.source_end)))))
                        (if (and item.source_start item.source_end)
                            (let [nonliteral (or item.selection_marker
                                                 (not= (- item.source_end
                                                          item.source_start)
                                                       (length item.text)))
                                  first (+ item.source_start
                                           (if nonliteral
                                               0
                                               at))
                                  last (if nonliteral
                                           item.source_end
                                           (+ item.source_start after))]
                              (set active
                                   (and (or item.source item.selection_marker)
                                        (< first selected.last)
                                        (> last selected.first))))
                            (when item.source
                              (set active
                                   (or whole
                                       (and line.source_start line.source_end
                                            (<= selected.first
                                                line.source_start)
                                            (>= selected.last line.source_end))))
                              (let [found (text:find piece (+ cursor 1) true)]
                                (when (and found
                                           (< (- found 1)
                                              (or line.source_end (length text))))
                                  (set active
                                       (and (< (- found 1) selected.last)
                                            (> (+ (- found 1) (length piece))
                                               selected.first)))
                                  (set cursor (+ (- found 1) (length piece)))))))
                        (when (not= marked active)
                          (flush)
                          (set marked active))
                        (when (= (length parts) 0)
                          (set parts-start at))
                        (tset parts (+ (length parts) 1) piece)
                        (set at after)))
                    (flush))
                  (set line.spans spans)
                  (when line.selected
                    (set line.selection_id id)
                    (set anchored true))
                  (table.insert decorated line))))
            ;; Whitespace-only or nonliteral content can have
            ;; no painted span and still be a navigation target.
            (when (and (not anchored) (. decorated 1))
              (set (. decorated 1 :selection_anchor) true)
              (set (. decorated 1 :selection_id) id))
            decorated)))))

(fn route-terminal-input [db event]
  (when (and db.selection (not db.picker) (not db.dialog)
             (not (. scrolling-inputs event.kind)))
    {:type :selection/action
     :action (or (misa.keybindings.action :selection event) :ignore)}))

(fn on-selection-open [db]
  (let [documents {}
        origins {}
        ids {}]
    (each [id (pairs (misa.catalog :selection-sources))]
      (tset ids (+ (length ids) 1) id))
    (table.sort ids)
    (each [_ id (ipairs ids)]
      (each [_ document (ipairs ((. (misa.catalog :selection-sources) id
                                    :documents) db))]
        (tset origins document.id id)
        (tset documents (+ (length documents) 1) document)))
    (let [frame {:index (math.max 1 (length documents)) :nodes documents}
          current (. frame.nodes frame.index)
          indices {}]
      (each [index document (ipairs documents)]
        (assert (and (= (type document.id) :string)
                     (not (. indices document.id)))
                "selection documents require unique string IDs")
        (tset indices document.id index))
      {:patch {:selection (misa.replace {:anchor current
                                         :anchor_document frame.index
                                         : indices
                                         :documents documents
                                         :sources origins
                                         :frames [frame]})}
       :fx [{:type :terminal/read}]})))

(fn on-selection-action [db event cofx]
  (let [state db.selection
        handler (. (misa.catalog :selection-actions) event.action)
        result (and state handler (handler state db event cofx))
        fx [{:type :terminal/read}]]
    (each [_ effect (ipairs (or (and result result.fx) []))]
      (table.insert fx effect))
    (let [next (or (and result result.state) state)]
      {:patch {:selection (if (and result result.close)
                              misa.delete
                              (and next (not= event.action :copy))
                              (misa.replace (misa.patch next
                                                        {:copied misa.delete}))
                              (misa.replace next))}
       :fx fx})))

(fn on-selection [db cofx]
  (let [state db.selection]
    (when cofx.projecting
      (misa.projections.publish :selection/geometry
                                (misa.selection.geometry db cofx.terminal)))
    (if (not state) nil (let [(current frame) (focus state)
                              document (. state.documents
                                          (. state.frames 1 :index))
                              path {}]
                          (each [_ entry (ipairs state.frames)]
                            (let [item (. entry.nodes entry.index)]
                              (when item
                                (tset path (+ (length path) 1) item.label))))
                          (let [hints {}]
                            (each [_ action (ipairs [:left
                                                     :right
                                                     :up
                                                     :down
                                                     :parent
                                                     :child
                                                     :copy
                                                     :visual
                                                     :close])]
                              (tset hints (+ (length hints) 1)
                                    {:action (.. :selection. action)
                                     :key (misa.keybindings.hint :selection
                                                                 action)
                                     :label action}))
                            (let [rendered (misa.components.render db
                                                                   :selection
                                                                   {:copied state.copied
                                                                    :visual state.visual
                                                                    : hints
                                                                    :index frame.index
                                                                    :nodes frame.nodes
                                                                    : path
                                                                    :text (or (and current
                                                                                   (document.text:sub (+ current.first
                                                                                                         1)
                                                                                                      current.last))
                                                                              "")}
                                                                   {:available_lines (or cofx.available_lines
                                                                                         cofx.terminal.lines)
                                                                    :columns cofx.terminal.columns})]
                              (set rendered.dock :input)
                              (set rendered.input_disabled true)
                              rendered))))))

(fn copy [state db]
  (let [slices (misa.selection.ranges db)]
    (when (> (length slices) 0)
      {:state (misa.patch state {:copied true})
       :fx [{:type :dispatch
             :event {:type :clipboard/copy
                     :text (table.concat (icollect [_ selected (ipairs slices)]
                                           (selected.text:sub (+ selected.first
                                                                 1)
                                                              selected.last))
                                         "\n\n")}}]})))

(fn visual [state]
  (let [(_ frame) (focus state)]
    {:state (misa.patch (move state frame.index false)
                        {:visual (not state.visual)})}))

(fn parent [state]
  {:state (ascend (misa.patch state {:preferred_column misa.delete}))})

(fn child [state]
  {:state (descend (misa.patch state {:preferred_column misa.delete}))})

(fn selection-open? [db]
  (and (not= db.selection nil) (not db.picker)))

(fn selection-sources [_ source]
  (assert (and (= (type source) :table) (= (type source.documents) :function)
               (or (= source.layout nil) (= (type source.layout) :function)))
          "selection source requires documents and optional layout"))

(fn selection-actions [_ handler]
  (assert (= (type handler) :function) "selection action must be a function"))

(fn build []
  "Build the declarations for selection."
  (let [declarations []
        actions {:close (fn [] {:state nil :close true})
                 :child child
                 :parent parent
                 :visual visual
                 :copy copy}]
    (each [_ direction (ipairs [:left :right :up :down])]
      (tset actions direction
            (fn [state db _ cofx]
              (let [document (. state.documents (. state.frames 1 :index))
                    accepted (and cofx cofx.presentation
                                  (. cofx.presentation :selection/geometry))
                    geometry (and accepted document (= accepted.id document.id)
                                  (= accepted.text document.text)
                                  (= accepted.source_part document.source_part)
                                  accepted.segments)]
                {:state (directional state direction
                                     (when geometry
                                       (fn [node] (geometry-at geometry node))))}))))
    (each [name motion (pairs motions)]
      (tset actions name
            (fn [state]
              (let [(_ frame) (focus state)]
                {:state (move state (motion frame)
                              (or state.visual (= name :extend_next)
                                  (= name :extend_previous)))}))))
    (table.insert declarations
                  {:catalog :services
                   :id :selection.geometry
                   :value selection-geometry})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :transcript/reset
                           :handler (fn [_] {:patch {:selection misa.delete}})}})
    (table.insert declarations
                  {:catalog :services
                   :id :selection.state
                   :value selection-state})
    (table.insert declarations
                  {:catalog :services
                   :id :selection.ranges
                   :value selection-ranges})
    ;; Decorate the existing rich transcript. Source-marked spans come from the
    ;; document renderer; chrome and table padding are never mistaken for content.
    (table.insert declarations
                  {:catalog :services
                   :id :selection.decorate
                   :value selection-decorate})
    (table.insert declarations
                  (let [definition {:action :select_transcript
                                    :context :global
                                    :default [:alt+s]}]
                    {:catalog :keybindings
                     :id (.. (. definition :context) "/" (. definition :action))
                     :value definition}))
    (table.insert declarations
                  (let [definition {:available (fn [db] (not db.picker))
                                    :binding {:action :select_transcript
                                              :context :global}
                                    :event {:type :selection/open}
                                    :id :selection.open
                                    :label "Navigate transcript"}]
                    {:catalog :actions
                     :id (. definition :id)
                     :value definition}))
    (let [keys {:child [:J :shift+j :enter]
                :close [:escape :ctrl_c :q]
                :copy [:y]
                :extend_next [:shift+arrow_down]
                :extend_previous [:shift+arrow_up]
                :first [:g]
                :last [:G]
                :next []
                :left [:h :arrow_left]
                :right [:l :arrow_right]
                :up [:k :arrow_up]
                :down [:j :arrow_down]
                :parent [:K :shift+k :backspace]
                :previous []
                :visual [:v]}]
      (each [_ action (ipairs [:left
                               :right
                               :up
                               :down
                               :previous
                               :next
                               :parent
                               :child
                               :copy
                               :extend_next
                               :extend_previous
                               :visual
                               :first
                               :last
                               :close])]
        (let [default (. keys action)]
          (table.insert declarations
                        (let [definition {: action
                                          :context :selection
                                          : default}]
                          {:catalog :keybindings
                           :id (.. (. definition :context) "/"
                                   (. definition :action))
                           :value definition}))
          (table.insert declarations
                        (let [definition {:available selection-open?
                                          :binding {: action
                                                    :context :selection}
                                          :event {: action
                                                  :type :selection/action}
                                          :id (.. :selection. action)
                                          :label (.. "Selection: " action)}]
                          {:catalog :actions
                           :id (. definition :id)
                           :value definition}))))
      (table.insert declarations
                    (let [definition {:id :selection/input
                                      :event :terminal/input
                                      :priority 500
                                      :context [:db/path]
                                      :resolve route-terminal-input}]
                      {:catalog :routes
                       :id (. definition :id)
                       :value definition}))
      (table.insert declarations
                    {:catalog :events
                     :value {:event :selection/open :handler on-selection-open}})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :selection/action
                             :handler on-selection-action}})
      (table.insert declarations
                    {:catalog :view-layers
                     :id :selection
                     :value {:handler on-selection}})
      (definitions.build :selection
        declarations
        {:selection-actions actions
         :validators {:selection-actions selection-actions
                      :selection-sources selection-sources}}))))

{: build}
