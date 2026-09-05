;; Modal editing policy over the editor's text/cursor model. Motions operate on

;; grapheme boundaries; neither terminal decoding nor the view knows Vim.

(fn line-start [text at]
  (- (or (: (text:sub 1 at) :match ".*\n()") 1) 1))

(fn line-end [text at]
  (- (or (text:find "\n" (+ at 1) true) (+ (length text) 1)) 1))

(fn prev [text at] (misa.layout.previous_boundary text at))

(fn next-at [text at] (misa.layout.next_boundary text at))

(fn class [text at]
  (let [c (text:sub (+ at 1) (next-at text at))]
    (if (= c "")
        :end
        (or (or (and (c:match "%s") :space)
                (and (or (c:match "[%w_]") (>= (or (c:byte) 0) 128)) :word))
            :punctuation))))

(fn motion [editor action]
  (var (text at) (values editor.text editor.cursor))
  (if (= action :left) (prev text at) (= action :right) (next-at text at)
      (= action :line_start) (line-start text at) (= action :line_end)
      (line-end text at) (or (= action :down) (= action :up))
      (let [start (line-start text at)
            column (misa.layout.width (text:sub (+ start 1) at))]
        (var target nil)
        (if (= action :down)
            (do
              (set target (+ (line-end text at) 1))
              (when (> target (length text)) (lua "return at")))
            (= start 0) (lua "return at")
            (set target (line-start text (- start 1))))
        (local finish (line-end text target))
        (local (_ bytes) (misa.layout.clip (text:sub (+ target 1) finish)
                                           column))
        (+ target bytes)) (= action :word_next)
      (let [group (class text at)]
        (while (and (< at (length text)) (= (class text at) group))
          (set at (next-at text at)))
        (while (and (< at (length text)) (= (class text at) :space))
          (set at (next-at text at)))
        at) (= action :word_previous)
      (do
        (set at (prev text at))
        (while (and (> at 0) (= (class text at) :space))
          (set at (prev text at)))
        (local group (class text at))
        (while (and (> at 0) (= (class text (prev text at)) group))
          (set at (prev text at)))
        at) (= action :word_end)
      (do
        (set at (next-at text at))
        (while (and (< at (length text)) (= (class text at) :space))
          (set at (next-at text at)))
        (local group (class text at))
        (while (and (< (next-at text at) (length text))
                    (= (class text (next-at text at)) group))
          (set at (next-at text at)))
        at) (= action :first) 0 (= action :last) (length text) nil))

(fn selection [editor state]
  (if (= state.anchor nil) nil
      (let [(first last) (values (math.min state.anchor editor.cursor)
                                 (math.max state.anchor editor.cursor))]
        (if state.linewise
            (values (line-start editor.text first)
                    (math.min (length editor.text)
                              (+ (line-end editor.text last) 1)))
            (values first (next-at editor.text last))))))

(fn reset-choice [editor]
  (set (editor.choice editor.choice_kind editor.choice_command
                      editor.choice_overlay) (values nil nil nil nil))
  nil)

(fn remember [state text cursor]
  (set state.undo (or state.undo {}))
  (tset state.undo (+ (length state.undo) 1) {: cursor : text})
  (while (> (length state.undo) 64) (table.remove state.undo 1))
  (set state.redo {})
  nil)

{:setup (fn [context]
          (local mode (or (. (or context.config.editing {}) :mode) :vim))
          (assert (or (= mode :vim) (= mode :plain))
                  "editing.mode must be vim or plain")
          (local enabled (= mode :vim))

          (fn reset-draft [db]
            (when db.editing
              (set (db.editing.anchor db.editing.operator
                                      db.editing.insert_group)
                   (values nil nil nil))
              (set (db.editing.undo db.editing.redo) (values {} {})))
            (when db.editor
              (set (db.editor.selection_start db.editor.selection_end)
                   (values nil nil)))
            {: db})

          (misa.reg_event :editor/restore reset-draft)
          (misa.reg_event :editor/steer reset-draft)
          (local bindings {:append [:a]
                           :append_end [:A]
                           :change [:c]
                           :delete [:d]
                           :delete_character [:x]
                           :down [:j :arrow_down]
                           :first [:g]
                           :insert [:i]
                           :insert_start [:I]
                           :last [:G]
                           :left [:h :arrow_left]
                           :line_end ["$"]
                           :line_start [:0]
                           :normal [:escape]
                           :open_above [:O]
                           :open_below [:o]
                           :paste [:p]
                           :redo [:U]
                           :right [:l :arrow_right]
                           :submit [:enter]
                           :undo [:u]
                           :up [:k :arrow_up]
                           :visual [:v]
                           :visual_line [:V]
                           :word_end [:e]
                           :word_next [:w]
                           :word_previous [:b]
                           :yank [:y]})

          (fn available [db] (and db.editor (not db.selection)))

          (local action-order {})
          (each [action (pairs bindings)]
            (tset action-order (+ (length action-order) 1) action))
          (table.sort action-order)
          (each [_ action (ipairs action-order)]
            (local keys (. bindings action))
            (misa.reg_keybinding {: action
                                  :context :editor.normal
                                  :default keys})
            (misa.reg_action {: available
                              :binding {: action :context :editor.normal}
                              :event {: action :type :editing/action}
                              :id (.. :editor. action)
                              :label (.. "Editor: " (action:gsub "_" " "))}))
          (misa.reg_interceptor {:after (fn [tx]
                                          (when (and (and (and (and tx.editing_input
                                                                    (= tx.db.editor.text
                                                                       ""))
                                                               (not= tx.editing_input.text
                                                                     ""))
                                                          (not= tx.event.type
                                                                :editing/action))
                                                     (not= tx.editing_input.kind
                                                           :backspace))
                                            (set (tx.db.editing.undo tx.db.editing.redo
                                                                     tx.db.editing.insert_group)
                                                 (values {} {} nil)))
                                          (when (and tx.editing_before
                                                     (not= tx.db.editor.text
                                                           tx.editing_before.text))
                                            (local state tx.db.editing)
                                            (when (not state.insert_group)
                                              (remember state
                                                        tx.editing_before.text
                                                        tx.editing_before.cursor)
                                              (set state.insert_group true)))
                                          tx)
                                 :before (fn [tx]
                                           (if (or (or (or (or (or (not= tx.event.type
                                                                         :terminal/input)
                                                                   (not enabled))
                                                               (not tx.cofx.terminal.interactive))
                                                           tx.db.dialog)
                                                       tx.db.picker)
                                                   tx.db.selection)
                                               tx
                                               (do
                                                 (local editor tx.db.editor)
                                                 (if (not editor) tx
                                                     (do
                                                       (set editor.mode
                                                            (or editor.mode
                                                                :insert))
                                                       (set tx.db.editing
                                                            (or tx.db.editing
                                                                {}))
                                                       (local state
                                                              tx.db.editing)
                                                       (set tx.editing_input
                                                            {:kind tx.event.kind
                                                             :text editor.text})
                                                       (if (= editor.mode
                                                              :insert)
                                                           (if (= tx.event.kind
                                                                  :escape)
                                                               (set tx.event
                                                                    {:action :normal
                                                                     :type :editing/action})
                                                               (or (or (= tx.event.kind
                                                                          :text)
                                                                       (= tx.event.kind
                                                                          :shift_enter))
                                                                   (= tx.event.kind
                                                                      :backspace))
                                                               (set tx.editing_before
                                                                    {:cursor editor.cursor
                                                                     :text editor.text}))
                                                           (do
                                                             (local action
                                                                    (misa.keybinding_action :editor.normal
                                                                                            tx.event))
                                                             (if (or (or (= tx.event.kind
                                                                            :ctrl_c)
                                                                         (= tx.event.kind
                                                                            :eof))
                                                                     (= tx.event.kind
                                                                        :ctrl_d))
                                                                 (do
                                                                   (set editor.mode
                                                                        :insert)
                                                                   (set state.anchor
                                                                        nil))
                                                                 (set tx.event
                                                                      {:action (or action
                                                                                   :ignore)
                                                                       :type :editing/action}))))
                                                       tx)))))
                                 :id :editing/input})
          (misa.reg_event :editing/action
                          (fn [db event]
                            (local editor db.editor)
                            (if (not editor)
                                {: db :fx [{:type :terminal/read}]}
                                (do
                                  (set db.editing (or db.editing {}))
                                  (local state db.editing)
                                  (local action event.action)
                                  (local fx [{:type :terminal/read}])
                                  (local (old-text old-cursor)
                                         (values editor.text editor.cursor))
                                  (reset-choice editor)
                                  (if (= action :normal)
                                      (do
                                        (when (and (= editor.mode :insert)
                                                   (> editor.cursor
                                                      (line-start editor.text
                                                                  editor.cursor)))
                                          (set editor.cursor
                                               (prev editor.text editor.cursor)))
                                        (set editor.mode :normal)
                                        (set (state.anchor state.operator
                                                           state.insert_group)
                                             (values nil nil nil)))
                                      (= action :submit)
                                      (do
                                        (set editor.mode :insert)
                                        (set state.anchor nil)
                                        (tset fx (+ (length fx) 1)
                                              {:event {:kind :enter
                                                       :type :terminal/input}
                                               :type :dispatch}))
                                      (or (or (or (or (or (= action :insert)
                                                          (= action :append))
                                                      (= action :insert_start))
                                                  (= action :append_end))
                                              (= action :open_below))
                                          (= action :open_above))
                                      (do
                                        (if (= action :append)
                                            (set editor.cursor
                                                 (next-at editor.text
                                                          editor.cursor))
                                            (= action :insert_start)
                                            (set editor.cursor
                                                 (line-start editor.text
                                                             editor.cursor))
                                            (= action :append_end)
                                            (set editor.cursor
                                                 (line-end editor.text
                                                           editor.cursor))
                                            (or (= action :open_below)
                                                (= action :open_above))
                                            (do
                                              (set editor.cursor
                                                   (or (and (= action
                                                               :open_below)
                                                            (line-end editor.text
                                                                      editor.cursor))
                                                       (line-start editor.text
                                                                   editor.cursor)))
                                              (set editor.text
                                                   (.. (editor.text:sub 1
                                                                        editor.cursor)
                                                       "\n"
                                                       (editor.text:sub (+ editor.cursor
                                                                           1))))
                                              (when (= action :open_below)
                                                (set editor.cursor
                                                     (+ editor.cursor 1)))))
                                        (set editor.mode :insert)
                                        (set (state.anchor state.operator
                                                           state.insert_group)
                                             (values nil nil nil)))
                                      (or (= action :visual)
                                          (= action :visual_line))
                                      (do
                                        (set editor.mode :visual)
                                        (set state.anchor editor.cursor)
                                        (set state.linewise
                                             (= action :visual_line)))
                                      (or (= action :undo) (= action :redo))
                                      (do
                                        (local from
                                               (or (and (= action :undo) :undo)
                                                   :redo))
                                        (local to
                                               (or (and (= action :undo) :redo)
                                                   :undo))
                                        (tset state from (or (. state from) {}))
                                        (tset state to (or (. state to) {}))
                                        (local value
                                               (table.remove (. state from)))
                                        (when value
                                          (tset (. state to)
                                                (+ (length (. state to)) 1)
                                                {:cursor editor.cursor
                                                 :text editor.text})
                                          (set (editor.text editor.cursor)
                                               (values value.text value.cursor)))
                                        (set (state.anchor state.operator)
                                             (values nil nil))
                                        (set editor.mode :normal))
                                      (= action :paste)
                                      (do
                                        (local register (or db.clipboard {}))
                                        (var text (or register.text ""))
                                        (if (and register.linewise
                                                 (not= text ""))
                                            (do
                                              (local finish
                                                     (line-end editor.text
                                                               editor.cursor))
                                              (if (< finish
                                                     (length editor.text))
                                                  (do
                                                    (set editor.cursor
                                                         (+ finish 1))
                                                    (when (not= (text:sub (- 1))
                                                                "\n")
                                                      (set text (.. text "\n"))))
                                                  (do
                                                    (set editor.cursor finish)
                                                    (set text
                                                         (.. "\n"
                                                             (text:gsub "\n$"
                                                                        ""))))))
                                            (set editor.cursor
                                                 (next-at editor.text
                                                          editor.cursor)))
                                        (set editor.text
                                             (.. (editor.text:sub 1
                                                                  editor.cursor)
                                                 text
                                                 (editor.text:sub (+ editor.cursor
                                                                     1))))
                                        (set editor.cursor
                                             (+ editor.cursor (length text))))
                                      (do
                                        (local target (motion editor action))
                                        (var (first last)
                                             (selection editor state))
                                        (var operator state.operator)
                                        (var linewise
                                             (and (not= first nil)
                                                  (= state.linewise true)))
                                        (if target
                                            (if operator
                                                (do
                                                  (set (first last)
                                                       (values (math.min editor.cursor
                                                                         target)
                                                               (math.max editor.cursor
                                                                         target)))
                                                  (when (= action :word_end)
                                                    (set last
                                                         (next-at editor.text
                                                                  last))))
                                                (set editor.cursor target))
                                            (= action :delete_character)
                                            (do
                                              (set (first last)
                                                   (values editor.cursor
                                                           (next-at editor.text
                                                                    editor.cursor)))
                                              (set operator :delete))
                                            (or (or (= action :delete)
                                                    (= action :change))
                                                (= action :yank))
                                            (if first (set operator action)
                                                (= operator action)
                                                (do
                                                  (set linewise true)
                                                  (set (first last)
                                                       (values (line-start editor.text
                                                                           editor.cursor)
                                                               (math.min (length editor.text)
                                                                         (+ (line-end editor.text
                                                                                      editor.cursor)
                                                                            1)))))
                                                (set state.operator action))
                                            (not target)
                                            (set state.operator nil))
                                        (when (and first
                                                   (or (or (or operator
                                                               (= action
                                                                  :delete))
                                                           (= action :change))
                                                       (= action :yank)))
                                          (set operator (or operator action))
                                          (local text
                                                 (editor.text:sub (+ first 1)
                                                                  last))
                                          (tset fx (+ (length fx) 1)
                                                {:event {: linewise
                                                         : text
                                                         :type :clipboard/copy}
                                                 :type :dispatch})
                                          (when (not= operator :yank)
                                            (set editor.text
                                                 (.. (editor.text:sub 1 first)
                                                     (editor.text:sub (+ last 1))))
                                            (set editor.cursor first))
                                          (set editor.mode
                                               (or (and (= operator :change)
                                                        :insert)
                                                   :normal))
                                          (set (state.anchor state.operator
                                                             state.insert_group)
                                               (values nil nil nil)))))
                                  (when (and (and (not= editor.text old-text)
                                                  (not= action :undo))
                                             (not= action :redo))
                                    (remember state old-text old-cursor))
                                  (set (editor.selection_start editor.selection_end)
                                       (selection editor state))
                                  {: db : fx}))))
          nil)}

