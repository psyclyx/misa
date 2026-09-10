;; Immutable editor transitions. Choice sessions own filtering and navigation;
;; the editor owns draft text, cursor, attachments, and submission effects.

(fn previous-cursor [text cursor]
  "Return the preceding grapheme boundary."
  (misa.layout.previous-boundary text cursor))

(fn next-cursor [text cursor]
  "Return the following grapheme boundary."
  (misa.layout.next-boundary text cursor))

(fn state [db]
  (let [editor (or db.editor {})
        text (if (= (type editor.text) :string) editor.text "")
        cursor (if (and (= (type editor.cursor) :number)
                        (= (% editor.cursor 1) 0) (>= editor.cursor 0)
                        (<= editor.cursor (length text)))
                   (misa.layout.boundary-at-or-before text editor.cursor)
                   (length text))]
    (misa.patch editor {: text
                        : cursor
                        :mode (or editor.mode :insert)
                        :busy (= editor.busy true)})))

(fn updated [editor fx reason]
  (values {:patch {:editor (misa.replace editor)}
           :fx (or fx [{:type :terminal/read}])}
          (or reason :preserve)))

(fn accounted [db result reason cofx]
  (if (or (not result) (= reason :preserve)
          (not (and misa.editor misa.editor.transition)))
      result
      (let [previous (state db)
            editor (. (misa.patch db (or result.patch {})) :editor)
            editing (or db.editing {})
            next (misa.editor.transition editing previous editor reason
                                         (and cofx cofx.terminal.interactive))]
        (if (= next editing) result
            (let [patch (collect [key value (pairs (or result.patch {}))]
                          key
                          value)]
              (tset patch :editing (misa.replace next))
              {: patch :fx result.fx})))))

(fn editor-db [db editor]
  (misa.patch db {:editor (misa.replace editor)}))

(fn dispatch-input [editor event cofx]
  (updated editor (if cofx.terminal.interactive
                      [{: event :type :dispatch} {:type :terminal/read}]
                      [{: event :type :dispatch}]) :discard))

(fn command-input [text]
  (let [(name args) (text:match "^(%S+)%s*(.-)%s*$")]
    (values (and name (misa.commands.lookup name)) args)))

(fn command-items []
  (let [result {}]
    (each [_ command (ipairs (misa.commands.all))]
      (table.insert result {:description command.description
                            :label command.name
                            :search command.search
                            :value command.name}))
    result))

(fn clear-choice [editor]
  (misa.patch editor {:choice misa.delete
                      :choice_kind misa.delete
                      :choice_command misa.delete
                      :choice_overlay misa.delete}))

(fn with-text [editor text cursor]
  "Return an editor value with updated text and cursor."
  (misa.patch editor {: text
                      :cursor (or cursor (length text))
                      :dismissed_choice misa.delete}))

(fn sync-choice [previous original-db]
  (let [editor (clear-choice previous)]
    (if (or (not= editor.cursor (length editor.text))
            (not= (editor.text:sub 1 1) "/")
            (= editor.dismissed_choice editor.text))
        editor
        (let [db (editor-db original-db editor)
              (command-name args) (editor.text:match "^(%S+)%s+(.*)$")]
          (if command-name
              (let [command (misa.commands.lookup command-name)]
                (if (not command) editor
                    (let [all (misa.commands.choice-items command "" db)
                          items (if (and (= (length all) 0) (not= args ""))
                                    (misa.commands.choice-items command args db)
                                    all)]
                      (if (= (length items) 0) editor
                          (let [spec (misa.patch (misa.commands.choice-spec command
                                                                            args
                                                                            db)
                                                 {:items (misa.replace items)})]
                            (misa.patch editor
                                        {:choice (misa.replace (misa.choices.session spec
                                                                                     db))
                                         :choice_kind :argument
                                         :choice_command command.name}))))))
              (misa.patch editor
                          {:choice (misa.replace (if (and misa.picker
                                                          misa.picker.session)
                                                     (misa.picker.session db
                                                                          (editor.text:sub 2))
                                                     (misa.choices.session {:input_prefix "/"
                                                                            :items (command-items)
                                                                            :purpose :command-completion
                                                                            :query (editor.text:sub 2)
                                                                            :title :Commands}
                                                                           db)))
                           :choice_kind :command}))))))

(fn choice-text [editor]
  (if (and editor.choice.input_prefix (not= editor.choice.input_prefix ""))
      (.. editor.choice.input_prefix editor.choice.query)
      (= editor.choice_kind :argument)
      (.. editor.choice_command " " editor.choice.query)
      (.. "/" editor.choice.query)))

(fn accept-choice [editor item]
  (clear-choice (with-text editor
                  (if (= editor.choice_kind :argument)
                      (.. editor.choice_command " " item.value)
                      item.value))))

(fn submit-exact-choice [editor]
  (let [panel (and editor.choice (. editor.choice.panels 1))
        highlighted (and panel (. panel.items panel.highlight))]
    (if (= editor.choice_kind :command)
        (let [command (misa.commands.lookup editor.text)]
          (and command highlighted (= highlighted.value editor.text)
               (not (or command.completion command.complete))))
        (= editor.choice_kind :argument)
        (and (not= editor.choice.query "") highlighted
             (= highlighted.value editor.choice.query))
        false)))

(fn overlay [editor choose-view]
  (let [sequence (+ (or editor.choice_sequence 0) 1)
        token (.. "editor:" sequence)]
    (updated (misa.patch editor
                         {:choice_sequence sequence :choice_overlay token})
             [{:event {:choose_view (= choose-view true)
                       :completion :editor/choice-selected
                       :id :inline-choice
                       :session editor.choice
                       :title editor.choice.title
                       : token
                       :type :picker/open}
               :type :dispatch}])))

(fn update-dynamic-items [editor original-db]
  (if (or (not= editor.choice_kind :argument)
          (> (length editor.choice.stack) 0))
      editor
      (let [command (misa.commands.lookup editor.choice_command)
            db (editor-db original-db editor)]
        (if (not command) editor
            (let [matched (misa.commands.choice-items command
                                                      editor.choice.query db)
                  items (if (= (length matched) 0)
                            (misa.commands.choice-items command "" db)
                            matched)]
              (misa.patch editor
                          {:choice (misa.replace (misa.choices.set-items editor.choice
                                                                         items
                                                                         db))}))))))

(fn emptied [editor]
  "Clear the current editor draft."
  (clear-choice (misa.patch (with-text editor "" 0)
                            {:attachments (misa.replace {})})))

(fn selected [db event cofx]
  "Apply the accepted choice to the editor."
  (let [previous (state db)]
    (if (or (not= event.picker :inline-choice)
            (not= event.picker_token previous.choice_overlay)
            (not previous.choice))
        nil
        (let [editor (misa.patch previous {:choice_overlay misa.delete})]
          (var accepted nil)
          (when (not event.cancelled)
            (each [_ item (ipairs editor.choice.items) &until accepted]
              (when (= item.value event.value) (set accepted item))))
          (if (not accepted) (updated editor)
              (let [canonical (or accepted.invocation
                                  (and (= editor.choice_kind :argument)
                                       (misa.commands.canonical editor.choice_command
                                                                accepted.value)))]
                (if canonical
                    (dispatch-input (clear-choice (with-text editor "" 0))
                                    (assert (misa.commands.invocation canonical))
                                    cofx)
                    (let [next (accept-choice editor accepted)
                          command (misa.commands.lookup next.text)]
                      (updated (if (and command
                                        (or command.completion command.complete))
                                   (sync-choice (with-text next
                                                  (.. next.text " "))
                                                db)
                                   next))))))))))

(fn completion-layout [editor db room width]
  (if (and misa.choices misa.choices.completion-layout)
      (let [geometry (misa.choices.completion-layout editor.choice db width
                                                     room)]
        (values geometry geometry.targets))
      (let [columns (misa.choices.rows editor.choice db)
            panel (. editor.choice.panels 1)]
        (if (and misa.choices misa.choices.viewport)
            (let [viewport (misa.choices.viewport panel (. columns 1 :rows)
                                                  width room 1)]
              (values viewport viewport.targets))
            (let [rows {}
                  targets {}
                  first (misa.choices.first-index panel room)]
              (for [index first (math.min (length panel.items)
                                          (- (+ first room) 1))]
                (let [row (. columns 1 :rows index)
                      shortcut (.. :option_1_ (+ (length rows) 1))]
                  (set row.hotkey (and shortcut (misa.choices.hint shortcut)))
                  (tset rows (+ (length rows) 1) row)
                  (when shortcut
                    (tset targets shortcut (. panel.items index)))))
              (values {: rows} targets))))))

(fn accept-input [editor item db cofx reason]
  (if (and item.invocation misa.commands misa.commands.invocation)
      (dispatch-input (clear-choice (with-text editor "" 0))
                      (assert (misa.commands.invocation item.invocation)) cofx)
      (let [command (and (= editor.choice_kind :command)
                         (misa.commands.lookup item.value))]
        (if (and command (or command.completion command.complete))
            (updated (sync-choice (with-text editor (.. command.name " ")) db)
                     nil reason)
            (let [next (accept-choice editor item)
                  invocation (and misa.commands misa.commands.invocation
                                  (misa.commands.invocation next.text))]
              (if invocation (dispatch-input (with-text next "" 0) invocation
                                             cofx)
                  (updated next nil reason)))))))

(fn choice-input [editor event action original-db cofx]
  (let [db (editor-db original-db editor)
        (_ targets) (when (misa.choices.needs-targets? editor.choice event)
                      (let [input (misa.components.render db :editor.input
                                                          {:cursor editor.cursor
                                                           :mode editor.mode
                                                           :selection_start editor.selection_start
                                                           :selection_end editor.selection_end
                                                           :text editor.text}
                                                          {:columns cofx.terminal.columns})
                            room (if (and misa.ui misa.ui.completion-room)
                                     (misa.ui.completion-room db cofx.terminal
                                                              (length input.lines))
                                     5)]
                        (completion-layout editor db room
                                           (or cofx.terminal.columns 80))))
        result (misa.choices.input editor.choice
                                   {: action
                                    :kind event.kind
                                    :text event.text
                                    :key event.key
                                    : targets}
                                   db)
        next (misa.patch editor {:choice (misa.replace result.session)})
        reason (if (or (= event.kind :text) (= event.kind :backspace))
                   :insert
                   :preserve)]
    (if result.accepted (accept-input next result.accepted db cofx reason)
        (or result.open_overlay result.replace_view)
        (when (and misa.picker misa.picker.enabled?)
          (overlay next result.replace_view)) result.cancelled
        (updated (clear-choice (misa.patch next {:dismissed_choice next.text})))
        result.favorite
        (updated next [{:event {:scope next.choice.preference_scope
                                :type :preferences/toggle
                                :value result.favorite}
                        :type :dispatch}
                       {:type :terminal/read}]) result.consumed
        (let [refreshed (update-dynamic-items next db)
              edited (with-text refreshed (choice-text refreshed))]
          (updated (if (= edited.text "") (clear-choice edited) edited) nil
                   reason)) nil)))

(fn ctrl-d [editor]
  "Delete forward or signal empty input."
  (if (< editor.cursor (length editor.text))
      (misa.patch editor
                  {:text (.. (editor.text:sub 1 editor.cursor)
                             (editor.text:sub (+ (next-cursor editor.text
                                                              editor.cursor)
                                                 1)))})
      editor))

(fn backspace [editor]
  "Remove the preceding character from the active input."
  (let [previous (previous-cursor editor.text editor.cursor)]
    (with-text editor
      (.. (editor.text:sub 1 previous) (editor.text:sub (+ editor.cursor 1)))
      previous)))

(fn insert-text [editor event]
  "Insert the received text at the active input position."
  (with-text editor
    (.. (editor.text:sub 1 editor.cursor) event.text
        (editor.text:sub (+ editor.cursor 1)))
    (+ editor.cursor (length event.text))))

(fn raw-input [editor event db cofx]
  (if (or (= event.kind :eof) (and (= event.kind :ctrl_d) (= editor.text "")))
      (updated editor [{:type :app/quit}])
      (and (= event.kind :enter)
           (or (not= editor.text "") (> (length (or editor.attachments {})) 0)))
      (let [(command args) (command-input editor.text)]
        (if (and (not command) (. (misa.sub db [:editor/lifecycle])
                                  :block_draft))
            (updated editor)
            (dispatch-input (emptied editor)
                            (if command
                                {:arguments args
                                 :command command.name
                                 :type :commands/invoke}
                                {:attachments editor.attachments
                                 :prompt editor.text
                                 :type (or (and misa.editor
                                                misa.editor.submit-event)
                                           :agent/submit)})
                            cofx)))
      (let [edit (. (misa.catalog :editor-edits) event.kind)]
        (updated (sync-choice (if edit (edit editor event) editor) db) nil
                 (if (= event.kind :ctrl_c)
                     :discard
                     ;; Forward-delete of the entire draft retains the existing
                     ;; discard behavior; partial forward deletes do not start a group.
                     (and (= event.kind :ctrl_d) (= editor.cursor 0)
                          (= (next-cursor editor.text 0) (length editor.text)))
                     :discard
                     (or (= event.kind :text) (= event.kind :backspace))
                     :insert
                     :preserve)))))

(fn terminal-input [db input cofx]
  "Route terminal input to choices or the draft editor."
  (let [previous (state db)
        event (if (= input.kind :shift_enter) {:kind :text :text "\n"} input)
        editor (if (= input.kind :shift_enter) (clear-choice previous) previous)
        command (and (= event.kind :enter)
                     (= editor.cursor (length editor.text))
                     (misa.commands.lookup editor.text))]
    (if (and (not (and editor.choice editor.choice.combo)) editor.busy
             (= event.kind :ctrl_c))
        (updated editor [{:event {:type :agent/cancel-active} :type :dispatch}
                         {:type :terminal/read}])
        (and (not editor.choice) command
             (or command.completion command.complete))
        (updated (sync-choice (with-text editor (.. command.name " ")) db))
        (and editor.choice (not editor.choice.combo)
             (= editor.choice_kind :command) (= event.kind :text)
             (event.text:find "%s"))
        (raw-input editor event db cofx)
        (let [action (when (not (and (= event.kind :enter)
                                     (submit-exact-choice editor)))
                       (misa.choices.action event))
              (result reason) (when (and editor.choice
                                         (or editor.choice.combo action
                                             (= event.kind :alt)
                                             (= event.kind :text)
                                             (= event.kind :backspace)))
                                (choice-input editor event action db cofx))]
          (if result (values result reason) (raw-input editor event db cofx))))))

(fn restore [db event]
  "Restore draft text, cursor, and attachments."
  (let [editor (state db)
        incoming (or event.text "")
        text (if event.replace incoming
                 (and (not= incoming "") (not= editor.text "")) (.. incoming
                                                                    "\n"
                                                                    editor.text)
                 (.. incoming editor.text))
        attachments {}]
    (each [_ item (ipairs (or event.attachments {}))]
      (table.insert attachments item))
    (when (not event.replace)
      (each [_ item (ipairs (or editor.attachments {}))]
        (table.insert attachments item)))
    (let [cursor (misa.layout.boundary-at-or-before text
                                                    (math.max 0
                                                              (math.min (or event.cursor
                                                                            (length text))
                                                                        (length text))))]
      (updated (clear-choice (misa.patch (with-text editor text cursor)
                                         {:mode :insert
                                          :selection_start misa.delete
                                          :selection_end misa.delete
                                          :attachments (misa.replace attachments)}))
               nil :restore))))

(fn attach [db event]
  "Add an attachment to the current draft."
  (let [editor (state db)
        attachments {}]
    (each [_ item (ipairs (or editor.attachments {}))]
      (table.insert attachments item))
    (when event.attachment (table.insert attachments event.attachment))
    (updated (misa.patch editor {:attachments (misa.replace attachments)}))))

(fn detach [db]
  "Remove an attachment from the current draft."
  (let [editor (state db)
        attachments {}]
    (for [index 1 (- (length (or editor.attachments {})) 1)]
      (tset attachments index (. editor.attachments index)))
    (updated (misa.patch editor {:attachments (misa.replace attachments)}))))

(fn compute-editor-lifecycle [inputs]
  "Combine lifecycle flags from editor contributors."
  (let [result {:hold_exit false :block_draft false}]
    (for [index 1 inputs.n]
      (let [flags (. inputs index)]
        (assert (= (type flags) :table)
                "editor lifecycle query must return flags")
        (each [key value (pairs flags)]
          (assert (and (or (= key :hold_exit) (= key :block_draft))
                       (= (type value) :boolean))
                  "invalid editor lifecycle flag")
          (when value
            (tset result key true)))))
    result))

(fn render-editor-project-input [db context]
  "Render editor input, reusing the accepted projection when unchanged."
  (let [editor db.editor]
    (misa.components.render db :editor.input
                            {:cursor editor.cursor
                             :mode editor.mode
                             :text editor.text
                             :selection_end editor.selection_end
                             :selection_start editor.selection_start}
                            context)))

(fn editor-layout [db projection-context]
  "Lay out editor input and completions within the supplied budgets."
  (let [editor (assert db.editor "editor state is not initialized")]
    (var completions {:rows {}})
    (let [terminal (and projection-context projection-context.terminal)
          input (misa.editor.project-input db
                                           {:columns (or (and terminal
                                                              terminal.columns)
                                                         80)})
          room (or (and projection-context projection-context.layout
                        (. (misa.ui.input-budgets projection-context.layout
                                                  (length input.lines)
                                                  projection-context.layout.dock_count)
                           :completions))
                   (and terminal misa.ui misa.ui.completion-room
                        (misa.ui.completion-room db terminal
                                                 (length input.lines)))
                   5)
          cursor (or input.cursor {:byte 0 :row 1 :shape :bar})]
      (when (and editor.choice (not editor.choice_overlay))
        (set completions (completion-layout editor db room
                                            (or (and terminal terminal.columns)
                                                80))))
      {:busy false
       :byte cursor.byte
       :shape cursor.shape
       :completions (. (misa.components.render db
                                               (or (and completions.columns
                                                        :picker)
                                                   :editor.completions)
                                               completions)
                       :lines)
       :input input.lines
       :row cursor.row})))

(fn lifecycle-inputs []
  "Return lifecycle subscription queries declared by editor contributors."
  (let [contributors (or (and misa.editor misa.editor.lifecycle) {})
        names (icollect [name (pairs contributors)]
                name)]
    (table.sort names)
    (icollect [_ name (ipairs names)]
      (. contributors name))))

(fn input-model [db]
  "Select the dependencies of the editor input projection."
  (let [editor (assert db.editor "editor state is not initialized")]
    {:cursor editor.cursor
     :mode editor.mode
     :text editor.text
     :selection_end editor.selection_end
     :selection_start editor.selection_start
     :components db.components
     :themes db.themes
     :hover_action db.hover_action
     :hover_link db.hover_link
     :choice_pending (and misa.choices misa.choices.pending
                          (misa.choices.pending db))}))

(fn editor-steer [db _ cofx]
  "Interrupt the agent and submit the current draft as steering input."
  (let [editor (state db)]
    (if (. (misa.sub db [:editor/lifecycle]) :block_draft)
        (updated editor)
        (let [result (dispatch-input (misa.patch (emptied editor)
                                                 {:mode :insert
                                                  :selection_start misa.delete
                                                  :selection_end misa.delete})
                                     {:attachments editor.attachments
                                      :prompt editor.text
                                      :type :queue/steer}
                                     cofx)]
          (values result :steer)))))

(fn editor-completion-check [db event cofx]
  "Resume input or exit when all editor work has settled."
  (let [editor db.editor
        pending (and cofx.terminal.interactive editor
                     (or (not= editor.text "")
                         (> (length (or editor.attachments {})) 0)))
        active (and db.agent
                    (or (not= db.agent.status :ready) db.agent.startup_prompt))]
    {:fx [(if (and event.exit (not active) (not pending)
                   (not (. (misa.sub db [:editor/lifecycle]) :hold_exit)))
              {:type :app/quit}
              cofx.terminal.interactive
              {:event {:type :ui/redraw} :type :dispatch}
              {:type :terminal/read})]}))

(fn agent-completed [_ event]
  "Schedule a completion check after the agent finishes."
  {:fx [{:type :dispatch
         :event {:type :editor/completion-check :exit event.exit}}]})

(fn agent-unavailable [_ event cofx]
  "Report an unavailable agent and resume input or exit."
  {:fx [{:event {:level :error :text event.message :type :transcript/harness}
         :type :dispatch}
        {:type (if cofx.terminal.interactive :terminal/read :app/quit)}]})

(fn agent-status [db event]
  "Update the editor busy state from agent status."
  (updated (misa.patch (state db) {:busy (not= event.status :ready)}) []))

(fn validate-transition [_ handler]
  "Validate an editor transition."
  (assert (= (type handler) :function) "transition must be a function"))

(fn app-start [config db _ cofx]
  "Initialize editor input and display a prompt when configured."
  (let [plain-prompt (= (. (or config.ui {}) :plain_prompt) true)]
    (let [fx {}]
      (when (= (length cofx.argv) 0)
        (when (and (not cofx.terminal.interactive) plain-prompt)
          (table.insert fx {:lines [{:spans [{:style {:foreground :default}
                                              :text "misa> enter a prompt:"}]}]
                            :type :view/commit}))
        (table.insert fx {:type :terminal/read}))
      (updated (state db) fx))))

(fn account-transition [handler db event cofx]
  "Account for an editor transition and its input activity."
  (let [(result reason) (handler db event cofx)]
    (accounted db result (or reason :preserve) cofx)))

{: agent-completed
 : agent-status
 : agent-unavailable
 : app-start
 : attach
 : compute-editor-lifecycle
 : detach
 : editor-completion-check
 : editor-layout
 : editor-steer
 : input-model
 : lifecycle-inputs
 : account-transition
 : render-editor-project-input
 : restore
 : selected
 : terminal-input
 : validate-transition
 : with-text
 : emptied
 : previous-cursor
 : backspace
 : insert-text
 : ctrl-d
 : next-cursor}
