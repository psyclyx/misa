;; Generic structural navigation over source documents supplied by features.

;; Selection state contains a frozen snapshot: streaming cannot move the range

;; between a user's navigation keystroke and copy.

(fn focus [state]
  (let [frame (. state.frames (length state.frames))]
    (values (. frame.nodes frame.index) frame)))

(fn frames-with [frames index frame]
  (icollect [i entry (ipairs frames)] (if (= i index) frame entry)))

(fn move [state index extending]
  (local depth (length state.frames))
  (local (_ frame) (focus state))
  (local next-frame (misa.patch frame {:index (math.max 1 (math.min (length frame.nodes) index))}))
  (local current (. next-frame.nodes next-frame.index))
  ;; Source offsets are local to a document; changing documents resets the anchor.
  (local anchor (if (and extending (> depth 1) (= state.anchor_depth depth))
                    state.anchor current))
  (misa.patch state {:frames (misa.replace (frames-with state.frames depth next-frame))
                     :anchor (misa.replace anchor) :anchor_depth depth
                     :range (misa.replace (when (and extending anchor current)
                                            {:first (math.min anchor.first current.first)
                                             :last (math.max anchor.last current.last)}))}))

(fn descend [state]
  (local current (focus state))
  (local document (. state.documents (. state.frames 1 :index)))
  (local children (and current (misa.selection_children document current)))
  (if (and children (> (length children) 0))
      (let [frames (icollect [_ frame (ipairs state.frames)] frame)]
        (table.insert frames {:index 1 :nodes children})
        (move (misa.patch state {:frames (misa.replace frames)}) 1 false))
      state))

(fn ascend [state]
  (if (> (length state.frames) 1)
      (let [frames (icollect [i frame (ipairs state.frames) &until (= i (length state.frames))] frame)
            next (misa.patch state {:frames (misa.replace frames)})]
        (move next (. frames (length frames) :index) false))
      state))

(local motions {:previous (fn [frame] (- frame.index 1))
                :next (fn [frame] (+ frame.index 1))
                :extend_previous (fn [frame] (- frame.index 1))
                :extend_next (fn [frame] (+ frame.index 1))
                :first (fn [] 1) :last (fn [frame] (length frame.nodes))})

{:setup (fn []
          (local setup-fx [])
          (local sources {})
          (local actions
                 {:close (fn [] {:state nil :close true})
                  :child (fn [state] {:state (descend state)})
                  :parent (fn [state] {:state (ascend state)})
                  :visual (fn [state]
                            (local (_ frame) (focus state))
                            {:state (misa.patch (move state frame.index false)
                                                {:visual (not state.visual)})})
                  :copy (fn [state db]
                          (local selected (misa.selection_projection db))
                          (when selected
                            {:state (misa.patch state {:copied true})
                             :fx [{:type :dispatch :event {:type :clipboard/copy
                                                          :text (selected.text:sub (+ selected.first 1) selected.last)}}]}))})
          (each [name motion (pairs motions)]
            (tset actions name
                  (fn [state]
                    (local (_ frame) (focus state))
                    {:state (move state (motion frame)
                                  (or state.visual (= name :extend_next) (= name :extend_previous)))})))
          (table.insert setup-fx
                        {:type :register/setup-effect :name :register/selection-action
                         :handler (fn [effect]
                                    (assert (and (= (type effect.id) :string)
                                                 (= (type effect.value) :function)
                                                 (not (. actions effect.id)))
                                            "invalid or duplicate selection action")
                                    (tset actions effect.id effect.value))})
          (table.insert setup-fx
                        {:type :register/setup-effect
                         :name :register/selection-source
                         :handler (fn [effect]
                                    (let [id effect.id
                                          source effect.value]
                                      (assert (and (not (. sources id))
                                                   (= (type source) :function))
                                              "invalid selection source")
                                      (tset sources id source)
                                      nil))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/reset
                         :handler (fn [_] {:patch {:selection misa.delete}})})
          (table.insert setup-fx
                        {:type :register/service
                         :name :selection_projection
                         :value (fn [db]
                                  (local state db.selection)
                                  (if (not state) nil
                                      (do
                                        (local current (focus state))
                                        (local document
                                               (. state.documents
                                                  (. state.frames 1 :index)))
                                        (if (and current document)
                                            (let [range (or state.range current)]
                                              {:first range.first
                                               :id document.id
                                               :kind current.kind
                                               :last range.last
                                               :text document.text})
                                            nil))))})
          ;; Decorate the existing rich transcript. Source-marked spans come from the
          ;; document renderer; chrome and table padding are never mistaken for content.
          (table.insert setup-fx
                        {:type :register/service
                         :name :selection_decorate
                         :value (fn [db id text lines]
                                  (local selected
                                         (misa.selection_projection db))
                                  (if (or (not selected) (not= selected.id id))
                                      lines
                                      (do
                                        (local highlight
                                               (misa.theme_style db :selection))
                                        (var (cursor block-start)
                                             (values 0 nil))
                                        (local decorated [])
                                        (each [_ original (ipairs lines)]
                                          (local line {})
                                          (each [key value (pairs original)] (tset line key value))
                                          (when (and (not= line.source_start
                                                           nil)
                                                     (not= line.source_start
                                                           block-start))
                                            (set (cursor block-start)
                                                 (values line.source_start
                                                         line.source_start)))
                                          (local spans {})
                                          (local whole
                                                 (and (= selected.first 0)
                                                      (= selected.last
                                                         (length selected.text))))
                                          (each [_ item (ipairs (or line.spans
                                                                    {}))]
                                            (var (at parts marked)
                                                 (values 0 {} nil))

                                            (fn flush []
                                              (if (= (length parts) 0) nil
                                                  (do
                                                    (local next {})
                                                    (each [key value (pairs item)]
                                                      (tset next key value))
                                                    (set next.text
                                                         (table.concat parts))
                                                    (when marked
                                                      (set next.style {})
                                                      (each [key value (pairs (or item.style
                                                                                  {}))]
                                                        (tset next.style key
                                                              value))
                                                      (each [key value (pairs highlight)]
                                                        (tset next.style key
                                                              value))
                                                      (set line.selected true))
                                                    (tset spans
                                                          (+ (length spans) 1)
                                                          next)
                                                    (set parts {})
                                                    nil)))

                                            (while (< at (length item.text))
                                              (local after
                                                     (misa.layout.next_boundary item.text
                                                                                at))
                                              (local piece
                                                     (item.text:sub (+ at 1)
                                                                    after))
                                              (var active whole)
                                              (when item.source
                                                (local found
                                                       (text:find piece
                                                                  (+ cursor 1)
                                                                  true))
                                                (when (and found
                                                           (< (- found 1)
                                                              (or line.source_end
                                                                  (length text))))
                                                  (set active
                                                       (and (< (- found 1)
                                                               selected.last)
                                                            (> (+ (- found 1)
                                                                  (length piece))
                                                               selected.first)))
                                                  (set cursor
                                                       (+ (- found 1)
                                                          (length piece)))))
                                              (when (not= marked active)
                                                (flush)
                                                (set marked active))
                                              (tset parts (+ (length parts) 1)
                                                    piece)
                                              (set at after))
                                            (flush))
                                          (set line.spans spans)
                                          (table.insert decorated line))
                                        decorated)))})
          (table.insert setup-fx
                        {:type :register/keybinding
                         :value {:action :select_transcript
                                 :context :global
                                 :default [:alt+s]}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:available (fn [db] (not db.picker))
                                 :binding {:action :select_transcript
                                           :context :global}
                                 :event {:type :selection/open}
                                 :id :selection.open
                                 :label "Navigate transcript"}})
          (local keys {:child [:l :arrow_right :enter]
                       :close [:escape :ctrl_c :q]
                       :copy [:y]
                       :extend_next [:shift+j :shift+arrow_down]
                       :extend_previous [:shift+k :shift+arrow_up]
                       :first [:g]
                       :last [:G]
                       :next [:j :arrow_down]
                       :parent [:h :arrow_left :backspace]
                       :previous [:k :arrow_up]
                       :visual [:v]})
          (each [_ action (ipairs [:previous
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
            (local default (. keys action))
            (table.insert setup-fx
                          {:type :register/keybinding
                           :value {: action :context :selection : default}})
            (table.insert setup-fx
                          {:type :register/action
                           :value {:available (fn [db]
                                                (and (not= db.selection nil)
                                                     (not db.picker)))
                                   :binding {: action :context :selection}
                                   :event {: action :type :selection/action}
                                   :id (.. :selection. action)
                                   :label (.. "Selection: " action)}}))
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (if (or (or (not= tx.event.type
                                                             :terminal/input)
                                                       tx.db.picker)
                                                   tx.db.dialog)
                                               tx
                                               (if (or (or (or (= tx.event.kind
                                                                  :wheel_up)
                                                               (= tx.event.kind
                                                                  :wheel_down))
                                                           (= tx.event.kind
                                                              :page_up))
                                                       (= tx.event.kind
                                                          :page_down))
                                                   tx
                                                   (do
                                                     (var next-event tx.event)
                                                     (if tx.db.selection
                                                         (set next-event
                                                              {:action (or (misa.keybinding_action :selection
                                                                                                   tx.event)
                                                                           :ignore)
                                                               :type :selection/action})
                                                         (= (misa.keybinding_action :global
                                                                                    tx.event)
                                                            :select_transcript)
                                                         (set next-event
                                                              {:type :selection/open}))
                                                     (misa.patch tx {:event (misa.replace next-event)})))))
                                 :id :selection/input}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :selection/open
                         :handler (fn [db]
                                    (local documents {})
                                    (local ids {})
                                    (each [id (pairs sources)]
                                      (tset ids (+ (length ids) 1) id))
                                    (table.sort ids)
                                    (each [_ id (ipairs ids)]
                                      (each [_ document (ipairs ((. sources id) db))]
                                        (tset documents
                                              (+ (length documents) 1) document)))
                                    (local frame {:index (math.max 1
                                                                  (length documents))
                                                  :nodes documents})
                                    (local current (. frame.nodes frame.index))
                                    {:patch {:selection (misa.replace
                                                          {:anchor current :anchor_depth 1
                                                           :documents documents :frames [frame]})}
                                     :fx [{:type :terminal/read}]})})
          (table.insert setup-fx
                        {:type :register/event :name :selection/action
                         :handler (fn [db event]
                                    (local state db.selection)
                                    (local handler (. actions event.action))
                                    (local result (and state handler (handler state db event)))
                                    (local fx [{:type :terminal/read}])
                                    (each [_ effect (ipairs (or (and result result.fx) []))]
                                      (table.insert fx effect))
                                    (local next (or (and result result.state) state))
                                    {:patch {:selection (if (and result result.close) misa.delete
                                                            (and next (not= event.action :copy))
                                                            (misa.replace (misa.patch next {:copied misa.delete}))
                                                            (misa.replace next))}
                                     :fx fx})})
          (table.insert setup-fx
                        {:type :register/view-layer
                         :id :selection
                         :handler (fn [db cofx]
                                    (local state db.selection)
                                    (if (not state) nil
                                        (do
                                          (local (current frame) (focus state))
                                          (local document
                                                 (. state.documents
                                                    (. state.frames 1 :index)))
                                          (local path {})
                                          (each [_ entry (ipairs state.frames)]
                                            (local item
                                                   (. entry.nodes entry.index))
                                            (when item
                                              (tset path (+ (length path) 1)
                                                    item.label)))
                                          (local hints {})
                                          (each [_ action (ipairs [:previous
                                                                   :next
                                                                   :parent
                                                                   :child
                                                                   :copy
                                                                   :visual
                                                                   :close])]
                                            (tset hints (+ (length hints) 1)
                                                  {:action (.. :selection.
                                                               action)
                                                   :key (misa.keybinding_hint :selection
                                                                              action)
                                                   :label action}))
                                          (local rendered
                                                 (misa.render_component db
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
                                                                         :columns cofx.terminal.columns}))
                                          (set rendered.dock :input)
                                          (set rendered.input_disabled true)
                                          rendered)))})
          {:fx setup-fx})}
