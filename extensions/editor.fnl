;; Multiline editor state. Text and cursor stay here; slash choices delegate all

;; filtering, navigation, view, and positional-key behavior to choices.fnl.

(fn previous-cursor [text cursor] (misa.layout.previous_boundary text cursor))

(fn next-cursor [text cursor] (misa.layout.next_boundary text cursor))

(fn state [db]
  (set db.editor (or db.editor {}))
  (local editor db.editor)
  (set editor.text (or (and (= (type editor.text) :string) editor.text) ""))
  (set editor.busy (= editor.busy true))
  (if (or (or (or (not= (type editor.cursor) :number)
                  (not= (% editor.cursor 1) 0))
              (< editor.cursor 0))
          (> editor.cursor (length editor.text)))
      (set editor.cursor (length editor.text))
      (set editor.cursor
           (misa.layout.boundary_at_or_before editor.text editor.cursor)))
  editor)

(fn dispatch-input [db event cofx]
  (let [fx [{: event :type :dispatch}]]
    (when cofx.terminal.interactive
      (tset fx (+ (length fx) 1) {:type :terminal/read}))
    {: db : fx}))

(fn command-input [text]
  (let [(name args) (text:match "^(%S+)%s*(.-)%s*$")]
    (values (and name (misa.command name)) args)))

(fn command-items []
  (let [result {}]
    (each [_ command (ipairs (misa.commands))]
      (tset result (+ (length result) 1)
            {:description command.description
             :label command.name
             :search command.search
             :value command.name}))
    result))

(fn clear-choice [editor]
  (set (editor.choice editor.choice_kind editor.choice_command
                      editor.choice_overlay) (values nil nil nil nil))
  nil)

(fn sync-choice [editor db]
  (clear-choice editor)
  (if (or (or (not= editor.cursor (length editor.text))
              (not= (editor.text:sub 1 1) "/"))
          (= editor.dismissed_choice editor.text)) nil
      (let [(command-name args) (editor.text:match "^(%S+)%s+(.*)$")]
        (if command-name
            (let [command (misa.command command-name)]
              (if (not command) (lua "return ")
                  (do
                    (var items (misa.command_choice_items command "" db))
                    (when (and (= (length items) 0) (not= args ""))
                      (set items (misa.command_choice_items command args db)))
                    (if (= (length items) 0) (lua "return ")
                        (let [spec (misa.command_choice_spec command args db)]
                          (set spec.items items)
                          (set editor.choice (misa.choice_session spec db))
                          (set (editor.choice_kind editor.choice_command)
                               (values :argument command.name)))))))
            (do
              (set editor.choice
                   (or (and misa.omnipicker_session
                            (misa.omnipicker_session db (editor.text:sub 2)))
                       (misa.choice_session {:input_prefix "/"
                                             :items (command-items)
                                             :purpose :command-completion
                                             :query (editor.text:sub 2)
                                             :title :Commands}
                                            db)))
              (set (editor.choice_kind editor.choice_command)
                   (values :command nil))))
        nil)))

(fn choice-text [editor]
  (if (and editor.choice.input_prefix (not= editor.choice.input_prefix ""))
      (.. editor.choice.input_prefix editor.choice.query)
      (if (= editor.choice_kind :argument)
          (.. editor.choice_command " " editor.choice.query)
          (.. "/" editor.choice.query))))

(fn sync-text-from-choice [editor] (set editor.text (choice-text editor))
  (set editor.cursor (length editor.text))
  (set editor.dismissed_choice nil)
  nil)

(fn accept-choice [editor item]
  (if (= editor.choice_kind :argument)
      (set editor.text (.. editor.choice_command " " item.value))
      (set editor.text item.value))
  (set editor.cursor (length editor.text))
  (set editor.dismissed_choice nil)
  (clear-choice editor)
  nil)

(fn highlighted-value [editor]
  (let [panel (and editor.choice (. editor.choice.panels 1))]
    (or (and (and panel (> panel.highlight 0))
             (. panel.items panel.highlight :value)) nil)))

(fn submit-exact-choice [editor]
  (if (= editor.choice_kind :command)
      (let [command (misa.command editor.text)]
        (and (not= command nil) (not (or command.completion command.complete))))
      (if (= editor.choice_kind :argument)
          (and (not= editor.choice.query "")
               (= (highlighted-value editor) editor.choice.query))
          false)))

(fn overlay-effect [editor choose-view]
  (set editor.choice_sequence (+ (or editor.choice_sequence 0) 1))
  (local token (.. "editor:" editor.choice_sequence))
  (set editor.choice_overlay token)
  {:event {:choose_view (= choose-view true)
           :completion :editor/choice-selected
           :id :inline-choice
           :session editor.choice
           :title editor.choice.title
           : token
           :type :picker/open}
   :type :dispatch})

(fn update-dynamic-items [editor db]
  (if (or (not= editor.choice_kind :argument)
          (> (length (or editor.choice.stack {})) 0)) nil
      (let [command (misa.command editor.choice_command)]
        (if (not command) nil (do
                                (var items
                                     (misa.command_choice_items command
                                                                editor.choice.query
                                                                db))
                                (when (= (length items) 0)
                                  (set items
                                       (misa.command_choice_items command "" db)))
                                (misa.choice_set_items editor.choice items db)
                                nil)))))

(fn completion-layout [editor db room width]
  (if misa.choice_completion_layout
      (let [geometry (misa.choice_completion_layout editor.choice db width room)]
        (values geometry geometry.targets))
      (let [columns (misa.choice_rows editor.choice db)
            panel (. editor.choice.panels 1)]
        (if misa.choice_viewport
            (let [viewport (misa.choice_viewport panel (. columns 1 :rows)
                                                 width room 1)]
              (values viewport viewport.targets))
            (let [(rows targets) (values {} {})
                  first (misa.choice_first_index panel room)]
              (for [index first (math.min (length panel.items)
                                          (- (+ first room) 1) (+ first 8))]
                (local row (. columns 1 :rows index))
                (set row.hotkey
                     (misa.choice_hint (.. :option_1_ (+ (length rows) 1))))
                (tset rows (+ (length rows) 1) row)
                (tset targets (.. :option_1_ (length rows))
                      (. panel.items index)))
              (values {: rows} targets))))))

{:setup (fn [context]
          (assert (and (and misa.choice_session misa.layout)
                       misa.layout.previous_boundary)
                  "editor requires choices and layout")
          (local config (or (and (= (type context.config) :table)
                                 context.config.ui)
                            nil))
          (local plain-prompt
                 (and (= (type config) :table) (= config.plain_prompt true)))

          (fn misa.editor_projection [db projection-context]
            (local editor (assert db.editor "editor state is not initialized"))
            (var completions {:rows {}})
            (local terminal
                   (and projection-context projection-context.terminal))
            (local input
                   (misa.render_component db :editor.input
                                          {:cursor editor.cursor
                                           :mode editor.mode
                                           :selection_end editor.selection_end
                                           :selection_start editor.selection_start
                                           :text editor.text}
                                          {:columns (or (and terminal
                                                             terminal.columns)
                                                        80)}))
            (local room (or (and (and terminal misa.inline_choice_room)
                                 (misa.inline_choice_room db terminal
                                                          (length input.lines)))
                            5))
            (when (and editor.choice (not editor.choice_overlay))
              (set completions
                   (completion-layout editor db room
                                      (or (and terminal terminal.columns) 80))))
            {:busy false
             :byte input.cursor.byte
             :completions (. (misa.render_component db
                                                    (or (and completions.columns
                                                             :picker)
                                                        :editor.completions)
                                                    completions)
                             :lines)
             :input input.lines
             :row input.cursor.row})

          (misa.reg_event :app/start
                          (fn [db _ cofx]
                            (state db)
                            (if (not= (length cofx.argv) 0) {: db}
                                (do
                                  (local fx {})
                                  (when (and (not cofx.terminal.interactive)
                                             plain-prompt)
                                    (tset fx (+ (length fx) 1)
                                          {:lines [{:spans [{:style {:foreground :default}
                                                             :text "misa> enter a prompt:"}]}]
                                           :type :view/commit}))
                                  (tset fx (+ (length fx) 1)
                                        {:type :terminal/read})
                                  {: db : fx}))))
          (misa.reg_event :agent/status
                          (fn [db event]
                            (tset (state db) :busy (not= event.status :ready))
                            {: db}))
          (misa.reg_event :agent/unavailable
                          (fn [db event cofx]
                            {: db
                             :fx [{:event {:level :error
                                           :text event.message
                                           :type :transcript/harness}
                                   :type :dispatch}
                                  (or (and cofx.terminal.interactive
                                           {:type :terminal/read})
                                      {:type :app/quit})]}))
          (misa.reg_event :agent/completed
                          (fn [db event cofx]
                            (if (and (and event.exit (not event.keep_alive))
                                     (not (and (and cofx.terminal.interactive
                                                    db.editor)
                                               (or (not= db.editor.text "")
                                                   (> (length (or db.editor.attachments
                                                                  {}))
                                                      0)))))
                                {: db :fx [{:type :app/quit}]}
                                {: db
                                 :fx [(or (and cofx.terminal.interactive
                                               {:event {:type :ui/redraw}
                                                :type :dispatch})
                                          {:type :terminal/read})]})))
          (misa.reg_event :editor/restore
                          (fn [db event]
                            (local editor (state db))
                            (local text (or event.text ""))
                            (set editor.text
                                 (or (and event.replace text)
                                     (or (and (and (not= text "")
                                                   (not= editor.text ""))
                                              (.. text "\n" editor.text))
                                         (.. text editor.text))))
                            (local attachments (or event.attachments {}))
                            (when (not event.replace)
                              (each [_ item (ipairs (or editor.attachments {}))]
                                (tset attachments (+ (length attachments) 1)
                                      item)))
                            (set editor.attachments attachments)
                            (set editor.cursor
                                 (math.min (or event.cursor
                                               (length editor.text))
                                           (length editor.text)))
                            (set editor.mode :insert)
                            (set editor.dismissed_choice nil)
                            (clear-choice editor)
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :editor/attach
                          (fn [db event]
                            (local editor (state db))
                            (set editor.attachments (or editor.attachments {}))
                            (tset editor.attachments
                                  (+ (length editor.attachments) 1)
                                  event.attachment)
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :editor/detach
                          (fn [db]
                            (table.remove (or (. (state db) :attachments) {}))
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :editor/steer
                          (fn [db _ cofx]
                            (local editor (state db))
                            (local (prompt attachments)
                                   (values editor.text editor.attachments))
                            (set (editor.text editor.cursor editor.attachments
                                              editor.mode)
                                 (values "" 0 {} :insert))
                            (clear-choice editor)
                            (dispatch-input db
                                            {: attachments
                                             : prompt
                                             :type :queue/steer}
                                            cofx)))
          (misa.reg_event :ui/redraw
                          (fn [db]
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :editor/choice-selected
                          (fn [db event cofx]
                            (local editor (state db))
                            (if (or (or (not= event.picker :inline-choice)
                                        (not= event.picker_token
                                              editor.choice_overlay))
                                    (not editor.choice))
                                nil
                                (do
                                  (set editor.choice_overlay nil)
                                  (when (not event.cancelled)
                                    (var accepted nil)
                                    (each [_ item (ipairs editor.choice.items)]
                                      (when (= item.value event.value)
                                        (set accepted item)
                                        (lua :break)))
                                    (when accepted
                                      (local canonical
                                             (or accepted.invocation
                                                 (and (= editor.choice_kind
                                                         :argument)
                                                      (misa.command_canonical editor.choice_command
                                                                              accepted.value))))
                                      (if canonical
                                          (do
                                            (set (editor.text editor.cursor)
                                                 (values "" 0))
                                            (clear-choice editor)
                                            (let [___antifnl_rtns_1___ [(dispatch-input db
                                                                                        (assert (misa.command_invocation canonical))
                                                                                        cofx)]]
                                              (lua "return (table.unpack or _G.unpack)(___antifnl_rtns_1___)")))
                                          (do
                                            (accept-choice editor accepted)
                                            (local command
                                                   (misa.command editor.text))
                                            (when (and command
                                                       (or command.completion
                                                           command.complete))
                                              (set editor.text
                                                   (.. editor.text " "))
                                              (set editor.cursor
                                                   (length editor.text))
                                              (sync-choice editor db))))))
                                  {: db :fx [{:type :terminal/read}]}))))
          (misa.reg_event :terminal/input
                          (fn [db event cofx]
                            (local editor (state db))
                            (if (and editor.busy (= event.kind :ctrl_c))
                                {: db
                                 :fx [{:event {:type :agent/cancel-active}
                                       :type :dispatch}
                                      {:type :terminal/read}]}
                                (do
                                  (when (= event.kind :shift_enter)
                                    (set-forcibly! event
                                                   {:kind :text :text "\n"})
                                    (clear-choice editor))
                                  ;; Enter on an exact command opens the same argument session as Space.
                                  ;; It must not execute a separate modal command flow or a recent invocation
                                  ;; that happens to be highlighted beside the command itself.
                                  (when (and (= event.kind :enter)
                                             (= editor.cursor
                                                (length editor.text)))
                                    (local command (misa.command editor.text))
                                    (when (and command
                                               (or command.completion
                                                   command.complete))
                                      (set editor.text (.. command.name " "))
                                      (set editor.cursor (length editor.text))
                                      (set editor.dismissed_choice nil)
                                      (sync-choice editor db)
                                      (let [___antifnl_rtn_1___ {: db
                                                                 :fx [{:type :terminal/read}]}]
                                        (lua "return ___antifnl_rtn_1___"))))
                                  ;; Whitespace changes a command-name completion into its argument session.
                                  (if (and (and (and editor.choice
                                                     (= editor.choice_kind
                                                        :command))
                                                (= event.kind :text))
                                           (event.text:find "%s"))
                                      (do
                                        (set editor.text
                                             (.. (editor.text:sub 1
                                                                  editor.cursor)
                                                 event.text
                                                 (editor.text:sub (+ editor.cursor
                                                                     1))))
                                        (set editor.cursor
                                             (+ editor.cursor
                                                (length event.text)))
                                        (set editor.dismissed_choice nil)
                                        (sync-choice editor db)
                                        {: db :fx [{:type :terminal/read}]})
                                      (do
                                        (var action (misa.choice_action event))
                                        (when (and (= event.kind :enter)
                                                   (submit-exact-choice editor))
                                          (set action nil))
                                        (when (and editor.choice
                                                   (or (or action
                                                           (= event.kind :text))
                                                       (= event.kind :backspace)))
                                          (local input-count
                                                 (length (. (misa.render_component db
                                                                                   :editor.input
                                                                                   {:cursor editor.cursor
                                                                                    :mode editor.mode
                                                                                    :selection_end editor.selection_end
                                                                                    :selection_start editor.selection_start
                                                                                    :text editor.text}
                                                                                   {:columns cofx.terminal.columns})
                                                            :lines)))
                                          (local room
                                                 (or (and misa.inline_choice_room
                                                          (misa.inline_choice_room db
                                                                                   cofx.terminal
                                                                                   input-count))
                                                     5))
                                          (local (_ targets)
                                                 (completion-layout editor db
                                                                    room
                                                                    (or cofx.terminal.columns
                                                                        80)))
                                          (local item (. targets action))
                                          (local result
                                                 (or (and item
                                                          (misa.choice_accept editor.choice
                                                                              item
                                                                              db))
                                                     (misa.choice_input editor.choice
                                                                        {: action
                                                                         :kind event.kind
                                                                         :text event.text}
                                                                        db)))
                                          (if result.accepted
                                              (if (and result.accepted.invocation
                                                       misa.command_invocation)
                                                  (do
                                                    (local invocation
                                                           (assert (misa.command_invocation result.accepted.invocation)))
                                                    (set (editor.text editor.cursor
                                                                      editor.dismissed_choice)
                                                         (values "" 0 nil))
                                                    (clear-choice editor)
                                                    (let [___antifnl_rtns_1___ [(dispatch-input db
                                                                                                invocation
                                                                                                cofx)]]
                                                      (lua "return (table.unpack or _G.unpack)(___antifnl_rtns_1___)")))
                                                  (do
                                                    (local command
                                                           (or (and (= editor.choice_kind
                                                                       :command)
                                                                    (misa.command result.accepted.value))
                                                               nil))
                                                    (if (and command
                                                             (or command.completion
                                                                 command.complete))
                                                        (do
                                                          (set editor.text
                                                               (.. command.name
                                                                   " "))
                                                          (set editor.cursor
                                                               (length editor.text))
                                                          (sync-choice editor
                                                                       db)
                                                          (let [___antifnl_rtn_1___ {: db
                                                                                     :fx [{:type :terminal/read}]}]
                                                            (lua "return ___antifnl_rtn_1___")))
                                                        (do
                                                          (accept-choice editor
                                                                         result.accepted)
                                                          (when (and item
                                                                     misa.command_invocation)
                                                            (local invocation
                                                                   (misa.command_invocation editor.text))
                                                            (when invocation
                                                              (set (editor.text editor.cursor)
                                                                   (values "" 0))
                                                              (let [___antifnl_rtns_1___ [(dispatch-input db
                                                                                                          invocation
                                                                                                          cofx)]]
                                                                (lua "return (table.unpack or _G.unpack)(___antifnl_rtns_1___)"))))
                                                          (let [___antifnl_rtn_1___ {: db
                                                                                     :fx [{:type :terminal/read}]}]
                                                            (lua "return ___antifnl_rtn_1___"))))))
                                              (or result.open_overlay
                                                  result.replace_view)
                                              (when misa.picker
                                                (let [___antifnl_rtn_1___ {: db
                                                                           :fx [(overlay-effect editor
                                                                                                result.replace_view)]}]
                                                  (lua "return ___antifnl_rtn_1___")))
                                              result.cancelled
                                              (do
                                                (set editor.dismissed_choice
                                                     editor.text)
                                                (clear-choice editor)
                                                (let [___antifnl_rtn_1___ {: db
                                                                           :fx [{:type :terminal/read}]}]
                                                  (lua "return ___antifnl_rtn_1___")))
                                              result.favorite
                                              (let [___antifnl_rtn_1___ {: db
                                                                         :fx [{:event {:scope editor.choice.preference_scope
                                                                                       :type :preferences/toggle
                                                                                       :value result.favorite}
                                                                               :type :dispatch}
                                                                              {:type :terminal/read}]}]
                                                (lua "return ___antifnl_rtn_1___"))
                                              result.consumed
                                              (do
                                                (update-dynamic-items editor db)
                                                (sync-text-from-choice editor)
                                                (when (= editor.text "")
                                                  (clear-choice editor))
                                                (let [___antifnl_rtn_1___ {: db
                                                                           :fx [{:type :terminal/read}]}]
                                                  (lua "return ___antifnl_rtn_1___")))))
                                        (if (= event.kind :text)
                                            (do
                                              (set editor.text
                                                   (.. (editor.text:sub 1
                                                                        editor.cursor)
                                                       event.text
                                                       (editor.text:sub (+ editor.cursor
                                                                           1))))
                                              (set editor.cursor
                                                   (+ editor.cursor
                                                      (length event.text)))
                                              (set editor.dismissed_choice nil))
                                            (= event.kind :backspace)
                                            (do
                                              (local previous
                                                     (previous-cursor editor.text
                                                                      editor.cursor))
                                              (set editor.text
                                                   (.. (editor.text:sub 1
                                                                        previous)
                                                       (editor.text:sub (+ editor.cursor
                                                                           1))))
                                              (set editor.cursor previous)
                                              (set editor.dismissed_choice nil))
                                            (= event.kind :arrow_left)
                                            (do
                                              (set editor.cursor
                                                   (previous-cursor editor.text
                                                                    editor.cursor))
                                              (set editor.dismissed_choice nil))
                                            (= event.kind :arrow_right)
                                            (do
                                              (set editor.cursor
                                                   (next-cursor editor.text
                                                                editor.cursor))
                                              (set editor.dismissed_choice nil))
                                            (and (= event.kind :enter)
                                                 (or (not= editor.text "")
                                                     (> (length (or editor.attachments
                                                                    {}))
                                                        0)))
                                            (do
                                              (local (prompt attachments)
                                                     (values editor.text
                                                             editor.attachments))
                                              (set (editor.text editor.cursor
                                                                editor.dismissed_choice
                                                                editor.attachments)
                                                   (values "" 0 nil {}))
                                              (clear-choice editor)
                                              (local (command args)
                                                     (command-input prompt))
                                              (let [___antifnl_rtns_1___ [(dispatch-input db
                                                                                          (or (and command
                                                                                                   {:arguments args
                                                                                                    :command command.name
                                                                                                    :type command.event})
                                                                                              {: attachments
                                                                                               : prompt
                                                                                               :type (or misa.submit_event
                                                                                                         :agent/submit)})
                                                                                          cofx)]]
                                                (lua "return (table.unpack or _G.unpack)(___antifnl_rtns_1___)")))
                                            (= event.kind :ctrl_c)
                                            (do
                                              (set (editor.text editor.cursor
                                                                editor.dismissed_choice
                                                                editor.attachments)
                                                   (values "" 0 nil {}))
                                              (clear-choice editor))
                                            (= event.kind :ctrl_d)
                                            (if (= editor.text "")
                                                (let [___antifnl_rtn_1___ {: db
                                                                           :fx [{:type :app/quit}]}]
                                                  (lua "return ___antifnl_rtn_1___"))
                                                (< editor.cursor
                                                   (length editor.text))
                                                (set editor.text
                                                     (.. (editor.text:sub 1
                                                                          editor.cursor)
                                                         (editor.text:sub (+ (next-cursor editor.text
                                                                                          editor.cursor)
                                                                             1)))))
                                            (= event.kind :eof)
                                            (let [___antifnl_rtn_1___ {: db
                                                                       :fx [{:type :app/quit}]}]
                                              (lua "return ___antifnl_rtn_1___")))
                                        (sync-choice editor db)
                                        {: db :fx [{:type :terminal/read}]}))))))
          nil)}

