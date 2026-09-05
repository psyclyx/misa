;; Generic structural navigation over source documents supplied by features.

;; Selection state contains a frozen snapshot: streaming cannot move the range

;; between a user's navigation keystroke and copy.

(fn focus [state]
  (let [frame (. state.frames (length state.frames))]
    (values (. frame.nodes frame.index) frame)))

{:setup (fn []
          (local sources {})
          (var sealed false)

          (fn misa.reg_selection_source [id source]
            (assert (and (and (not sealed) (not (. sources id)))
                         (= (type source) :function))
                    "invalid selection source")
            (tset sources id source)
            nil)

          (misa.reg_event :app/start (fn [db] (set sealed true) {: db}))
          (misa.reg_event :transcript/reset
                          (fn [db] (set db.selection nil) {: db}))

          (fn misa.selection_projection [db]
            (local state db.selection)
            (if (not state) nil (do
                                  (local current (focus state))
                                  (local document
                                         (. state.documents
                                            (. state.frames 1 :index)))
                                  (if (and current document)
                                      {:first current.first
                                       :id document.id
                                       :kind current.kind
                                       :last current.last
                                       :text document.text}
                                      nil))))

          ;; Decorate the existing rich transcript. Source-marked spans come from the
          ;; document renderer; chrome and table padding are never mistaken for content.

          (fn misa.selection_decorate [db id text lines]
            (local selected (misa.selection_projection db))
            (if (or (not selected) (not= selected.id id)) lines
                (do
                  (local highlight (misa.theme_style db :selection))
                  (var (cursor block-start) (values 0 nil))
                  (each [_ line (ipairs lines)]
                    (when (and (not= line.source_start nil)
                               (not= line.source_start block-start))
                      (set (cursor block-start)
                           (values line.source_start line.source_start)))
                    (local spans {})
                    (local whole
                           (and (= selected.first 0)
                                (= selected.last (length selected.text))))
                    (each [_ item (ipairs (or line.spans {}))]
                      (var (at parts marked) (values 0 {} nil))

                      (fn flush []
                        (if (= (length parts) 0) nil
                            (do
                              (local next {})
                              (each [key value (pairs item)]
                                (tset next key value))
                              (set next.text (table.concat parts))
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
                        (local after (misa.layout.next_boundary item.text at))
                        (local piece (item.text:sub (+ at 1) after))
                        (var active whole)
                        (when item.source
                          (local found (text:find piece (+ cursor 1) true))
                          (when (and found
                                     (< (- found 1)
                                        (or line.source_end (length text))))
                            (set active
                                 (and (< (- found 1) selected.last)
                                      (> (+ (- found 1) (length piece))
                                         selected.first)))
                            (set cursor (+ (- found 1) (length piece)))))
                        (when (not= marked active) (flush) (set marked active))
                        (tset parts (+ (length parts) 1) piece)
                        (set at after))
                      (flush))
                    (set line.spans spans))
                  lines)))

          (misa.reg_keybinding {:action :select_transcript
                                :context :global
                                :default [:alt+s]})
          (misa.reg_action {:available (fn [db] (not db.picker))
                            :binding {:action :select_transcript
                                      :context :global}
                            :event {:type :selection/open}
                            :id :selection.open
                            :label "Select and copy transcript"})
          (local keys {:child [:l :arrow_right :enter]
                       :close [:escape :ctrl_c :q]
                       :copy [:y]
                       :first [:g]
                       :last [:G]
                       :next [:j :arrow_down]
                       :parent [:h :arrow_left :backspace]
                       :previous [:k :arrow_up]})
          (each [_ action (ipairs [:previous
                                   :next
                                   :parent
                                   :child
                                   :copy
                                   :first
                                   :last
                                   :close])]
            (local default (. keys action))
            (misa.reg_keybinding {: action :context :selection : default})
            (misa.reg_action {:available (fn [db]
                                           (and (not= db.selection nil)
                                                (not db.picker)))
                              :binding {: action :context :selection}
                              :event {: action :type :selection/action}
                              :id (.. :selection. action)
                              :label (.. "Selection: " action)}))
          (misa.reg_interceptor {:before (fn [tx]
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
                                                     (if tx.db.selection
                                                         (set tx.event
                                                              {:action (or (misa.keybinding_action :selection
                                                                                                   tx.event)
                                                                           :ignore)
                                                               :type :selection/action})
                                                         (= (misa.keybinding_action :global
                                                                                    tx.event)
                                                            :select_transcript)
                                                         (set tx.event
                                                              {:type :selection/open}))
                                                     tx))))
                                 :id :selection/input})
          (misa.reg_event :selection/open
                          (fn [db]
                            (local documents {})
                            (local ids {})
                            (each [id (pairs sources)]
                              (tset ids (+ (length ids) 1) id))
                            (table.sort ids)
                            (each [_ id (ipairs ids)]
                              (each [_ document (ipairs ((. sources id) db))]
                                (tset documents (+ (length documents) 1)
                                      document)))
                            (set db.selection
                                 {: documents
                                  :frames [{:index (math.max 1
                                                             (length documents))
                                            :nodes documents}]})
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :selection/action
                          (fn [db event]
                            (local state db.selection)
                            (if (not state) {: db :fx [{:type :terminal/read}]}
                                (do
                                  (local (current frame) (focus state))
                                  (local action event.action)
                                  (local fx [{:type :terminal/read}])
                                  (if (= action :close) (set db.selection nil)
                                      (= action :previous)
                                      (set frame.index
                                           (math.max 1 (- frame.index 1)))
                                      (= action :next)
                                      (set frame.index
                                           (math.max 1
                                                     (math.min (length frame.nodes)
                                                               (+ frame.index 1))))
                                      (= action :first) (set frame.index 1)
                                      (= action :last)
                                      (set frame.index
                                           (math.max 1 (length frame.nodes)))
                                      (and (= action :parent)
                                           (> (length state.frames) 1))
                                      (table.remove state.frames)
                                      (and current (= action :child))
                                      (do
                                        (local document
                                               (. state.documents
                                                  (. state.frames 1 :index)))
                                        (local children
                                               (misa.selection_children document
                                                                        current))
                                        (when (> (length children) 0)
                                          (tset state.frames
                                                (+ (length state.frames) 1)
                                                {:index 1 :nodes children})))
                                      (and current (= action :copy))
                                      (do
                                        (local document
                                               (. state.documents
                                                  (. state.frames 1 :index)))
                                        (tset fx (+ (length fx) 1)
                                              {:event {:text (document.text:sub (+ current.first
                                                                                   1)
                                                                                current.last)
                                                       :type :clipboard/copy}
                                               :type :dispatch})
                                        (set state.copied true)))
                                  (when (not= action :copy)
                                    (set state.copied nil))
                                  {: db : fx}))))
          (misa.reg_view_layer :selection
                               (fn [db cofx]
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
                                                                :close])]
                                         (tset hints (+ (length hints) 1)
                                               {:action (.. :selection. action)
                                                :key (misa.keybinding_hint :selection
                                                                           action)
                                                :label action}))
                                       (local rendered
                                              (misa.render_component db
                                                                     :selection
                                                                     {:copied state.copied
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
                                       rendered))))
          nil)}

