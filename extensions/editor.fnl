;; Immutable editor transitions. Choice sessions own filtering and navigation;
;; the editor owns draft text, cursor, attachments, and submission effects.

(fn previous-cursor [text cursor] (misa.layout.previous_boundary text cursor))
(fn next-cursor [text cursor] (misa.layout.next_boundary text cursor))

(fn state [db]
  (local editor (or db.editor {}))
  (local text (if (= (type editor.text) :string) editor.text ""))
  (local cursor
         (if (and (= (type editor.cursor) :number) (= (% editor.cursor 1) 0)
                  (>= editor.cursor 0) (<= editor.cursor (length text)))
             (misa.layout.boundary_at_or_before text editor.cursor)
             (length text)))
  (misa.patch editor {: text : cursor :busy (= editor.busy true)}))

(fn updated [editor fx reason]
  (values {:patch {:editor (misa.replace editor)} :fx (or fx [{:type :terminal/read}])}
          (or reason :preserve)))

(fn accounted [db result reason cofx]
  (if (or (not result) (= reason :preserve) (not misa.editing_transition)) result
      (let [previous (state db)
            editor (. (misa.patch db (or result.patch {})) :editor)
            editing (or db.editing {})
            next (misa.editing_transition editing previous editor reason
                                         (and cofx cofx.terminal.interactive))]
        (if (= next editing) result
            (let [patch (collect [key value (pairs (or result.patch {}))] key value)]
              (tset patch :editing (misa.replace next))
              {: patch :fx result.fx})))))

(fn editor-db [db editor] (misa.patch db {:editor (misa.replace editor)}))

(fn dispatch-input [editor event cofx]
  (updated editor (if cofx.terminal.interactive
                      [{: event :type :dispatch} {:type :terminal/read}]
                      [{: event :type :dispatch}]) :discard))

(fn command-input [text]
  (local (name args) (text:match "^(%S+)%s*(.-)%s*$"))
  (values (and name (misa.command name)) args))

(fn command-items []
  (local result {})
  (each [_ command (ipairs (misa.commands))]
    (table.insert result {:description command.description :label command.name
                          :search command.search :value command.name}))
  result)

(fn clear-choice [editor]
  (misa.patch editor {:choice misa.delete :choice_kind misa.delete
                      :choice_command misa.delete :choice_overlay misa.delete}))

(fn with-text [editor text cursor]
  (misa.patch editor {: text :cursor (or cursor (length text)) :dismissed_choice misa.delete}))

(fn sync-choice [previous original-db]
  (local editor (clear-choice previous))
  (if (or (not= editor.cursor (length editor.text))
          (not= (editor.text:sub 1 1) "/")
          (= editor.dismissed_choice editor.text))
      editor
      (let [db (editor-db original-db editor)
            (command-name args) (editor.text:match "^(%S+)%s+(.*)$")]
        (if command-name
            (let [command (misa.command command-name)]
              (if (not command) editor
                  (let [all (misa.command_choice_items command "" db)
                        items (if (and (= (length all) 0) (not= args ""))
                                  (misa.command_choice_items command args db) all)]
                    (if (= (length items) 0) editor
                        (let [spec (misa.patch (misa.command_choice_spec command args db)
                                              {:items (misa.replace items)})]
                          (misa.patch editor {:choice (misa.replace (misa.choice_session spec db))
                                              :choice_kind :argument :choice_command command.name}))))))
            (misa.patch editor
                        {:choice (misa.replace
                                  (if misa.omnipicker_session
                                      (misa.omnipicker_session db (editor.text:sub 2))
                                      (misa.choice_session {:input_prefix "/" :items (command-items)
                                                            :purpose :command-completion
                                                            :query (editor.text:sub 2) :title :Commands} db)))
                         :choice_kind :command})))))

(fn choice-text [editor]
  (if (and editor.choice.input_prefix (not= editor.choice.input_prefix ""))
      (.. editor.choice.input_prefix editor.choice.query)
      (= editor.choice_kind :argument) (.. editor.choice_command " " editor.choice.query)
      (.. "/" editor.choice.query)))

(fn accept-choice [editor item]
  (clear-choice (with-text editor (if (= editor.choice_kind :argument)
                                    (.. editor.choice_command " " item.value) item.value))))

(fn submit-exact-choice [editor]
  (local panel (and editor.choice (. editor.choice.panels 1)))
  (local highlighted (and panel (. panel.items panel.highlight)))
  (if (= editor.choice_kind :command)
      (let [command (misa.command editor.text)]
        (and command (not (or command.completion command.complete))))
      (= editor.choice_kind :argument)
      (and (not= editor.choice.query "") highlighted (= highlighted.value editor.choice.query))
      false))

(fn overlay [editor choose-view]
  (local sequence (+ (or editor.choice_sequence 0) 1))
  (local token (.. "editor:" sequence))
  (updated (misa.patch editor {:choice_sequence sequence :choice_overlay token})
           [{:event {:choose_view (= choose-view true) :completion :editor/choice-selected
                     :id :inline-choice :session editor.choice :title editor.choice.title
                     : token :type :picker/open}
             :type :dispatch}]))

(fn update-dynamic-items [editor original-db]
  (if (or (not= editor.choice_kind :argument) (> (length editor.choice.stack) 0))
      editor
      (let [command (misa.command editor.choice_command)
            db (editor-db original-db editor)]
        (if (not command) editor
            (let [matched (misa.command_choice_items command editor.choice.query db)
                  items (if (= (length matched) 0) (misa.command_choice_items command "" db) matched)]
              (misa.patch editor {:choice (misa.replace (misa.choice_set_items editor.choice items db))}))))))

(fn emptied [editor]
  (clear-choice (misa.patch (with-text editor "" 0) {:attachments (misa.replace {})})))

(fn selected [db event cofx]
  (local previous (state db))
  (if (or (not= event.picker :inline-choice)
          (not= event.picker_token previous.choice_overlay) (not previous.choice))
      nil
      (let [editor (misa.patch previous {:choice_overlay misa.delete})]
        (var accepted nil)
        (when (not event.cancelled)
          (each [_ item (ipairs editor.choice.items) &until accepted]
            (when (= item.value event.value) (set accepted item))))
        (if (not accepted) (updated editor)
            (let [canonical (or accepted.invocation
                                (and (= editor.choice_kind :argument)
                                     (misa.command_canonical editor.choice_command accepted.value)))]
              (if canonical
                  (dispatch-input (clear-choice (with-text editor "" 0))
                                  (assert (misa.command_invocation canonical)) cofx)
                  (let [next (accept-choice editor accepted)
                        command (misa.command next.text)]
                    (updated (if (and command (or command.completion command.complete))
                                 (sync-choice (with-text next (.. next.text " ")) db) next)))))))))

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
                                          (- (+ first room) 1))]
                (local row (. columns 1 :rows index))
                (local shortcut
                       (when (< (length rows) 9)
                         (.. :option_1_ (+ (length rows) 1))))
                (set row.hotkey (and shortcut (misa.choice_hint shortcut)))
                (tset rows (+ (length rows) 1) row)
                (when shortcut
                  (tset targets shortcut (. panel.items index))))
              (values {: rows} targets))))))


(fn accept-input [editor item positional db cofx reason]
  (if (and item.invocation misa.command_invocation)
      (dispatch-input (clear-choice (with-text editor "" 0))
                      (assert (misa.command_invocation item.invocation)) cofx)
      (let [command (and (= editor.choice_kind :command) (misa.command item.value))]
        (if (and command (or command.completion command.complete))
            (updated (sync-choice (with-text editor (.. command.name " ")) db) nil reason)
            (let [next (accept-choice editor item)
                  invocation (and positional misa.command_invocation (misa.command_invocation next.text))]
              (if invocation (dispatch-input (with-text next "" 0) invocation cofx)
                  (updated next nil reason)))))))

(fn choice-input [editor event action original-db cofx]
  (local db (editor-db original-db editor))
  (local input (misa.render_component db :editor.input
                                     {:cursor editor.cursor :mode editor.mode
                                      :selection_start editor.selection_start :selection_end editor.selection_end
                                      :text editor.text}
                                     {:columns cofx.terminal.columns}))
  (local room (if misa.inline_choice_room
                  (misa.inline_choice_room db cofx.terminal (length input.lines)) 5))
  (local (_ targets) (completion-layout editor db room (or cofx.terminal.columns 80)))
  (local item (. targets action))
  (local result (if item (misa.choice_accept editor.choice item db)
                    (misa.choice_input editor.choice {: action :kind event.kind :text event.text} db)))
  (local next (misa.patch editor {:choice (misa.replace result.session)}))
  (local reason (if (or (= event.kind :text) (= event.kind :backspace)) :insert :preserve))
  (if result.accepted (accept-input next result.accepted item db cofx reason)
      (or result.open_overlay result.replace_view) (when misa.picker (overlay next result.replace_view))
      result.cancelled (updated (clear-choice (misa.patch next {:dismissed_choice next.text})))
      result.favorite (updated next [{:event {:scope next.choice.preference_scope
                                             :type :preferences/toggle :value result.favorite}
                                      :type :dispatch}
                                     {:type :terminal/read}])
      result.consumed
      (let [refreshed (update-dynamic-items next db)
            edited (with-text refreshed (choice-text refreshed))]
        (updated (if (= edited.text "") (clear-choice edited) edited) nil reason))
      nil))

(local edits
       {:text (fn [editor event]
                (with-text editor (.. (editor.text:sub 1 editor.cursor) event.text
                                      (editor.text:sub (+ editor.cursor 1)))
                           (+ editor.cursor (length event.text))))
        :backspace (fn [editor]
                     (local previous (previous-cursor editor.text editor.cursor))
                     (with-text editor (.. (editor.text:sub 1 previous)
                                           (editor.text:sub (+ editor.cursor 1))) previous))
        :arrow_left (fn [editor]
                      (with-text editor editor.text (previous-cursor editor.text editor.cursor)))
        :arrow_right (fn [editor]
                       (with-text editor editor.text (next-cursor editor.text editor.cursor)))
        :ctrl_c emptied
        :ctrl_d (fn [editor]
                  (if (< editor.cursor (length editor.text))
                      (misa.patch editor
                                  {:text (.. (editor.text:sub 1 editor.cursor)
                                             (editor.text:sub (+ (next-cursor editor.text editor.cursor) 1)))})
                      editor))})

(fn raw-input [editor event db cofx]
  (if (or (= event.kind :eof) (and (= event.kind :ctrl_d) (= editor.text "")))
      (updated editor [{:type :app/quit}])
      (and (= event.kind :enter)
           (or (not= editor.text "") (> (length (or editor.attachments {})) 0)))
      (let [(command args) (command-input editor.text)]
        (if (and (not command) (. (misa.sub db [:editor/lifecycle]) :block_draft))
            (updated editor)
            (dispatch-input (emptied editor)
                            (if command {:arguments args :command command.name :type command.event}
                                {:attachments editor.attachments :prompt editor.text
                                 :type (or misa.submit_event :agent/submit)}) cofx)))
      (let [edit (. edits event.kind)]
        (updated (sync-choice (if edit (edit editor event) editor) db) nil
                 (if (= event.kind :ctrl_c) :discard
                     ;; Forward-delete of the entire draft retains the existing
                     ;; discard behavior; partial forward deletes do not start a group.
                     (and (= event.kind :ctrl_d) (= editor.cursor 0)
                          (= (next-cursor editor.text 0) (length editor.text))) :discard
                     (or (= event.kind :text) (= event.kind :backspace)) :insert
                     :preserve)))))

(fn terminal-input [db input cofx]
  (local previous (state db))
  (local event (if (= input.kind :shift_enter) {:kind :text :text "\n"} input))
  (local editor (if (= input.kind :shift_enter) (clear-choice previous) previous))
  (local command (and (= event.kind :enter) (= editor.cursor (length editor.text))
                      (misa.command editor.text)))
  (if (and editor.busy (= event.kind :ctrl_c))
      (updated editor [{:event {:type :agent/cancel-active} :type :dispatch} {:type :terminal/read}])
      (and command (or command.completion command.complete))
      (updated (sync-choice (with-text editor (.. command.name " ")) db))
      (and editor.choice (= editor.choice_kind :command)
           (= event.kind :text) (event.text:find "%s"))
      (raw-input editor event db cofx)
      (let [action (when (not (and (= event.kind :enter) (submit-exact-choice editor)))
                     (misa.choice_action event))]
        (local (result reason)
               (when (and editor.choice (or action (= event.kind :text) (= event.kind :backspace)))
                 (choice-input editor event action db cofx)))
        (if result (values result reason) (raw-input editor event db cofx)))))

(fn restore [db event]
  (local editor (state db))
  (local incoming (or event.text ""))
  (local text (if event.replace incoming
                  (and (not= incoming "") (not= editor.text "")) (.. incoming "\n" editor.text)
                  (.. incoming editor.text)))
  (local attachments {})
  (each [_ item (ipairs (or event.attachments {}))] (table.insert attachments item))
  (when (not event.replace)
    (each [_ item (ipairs (or editor.attachments {}))] (table.insert attachments item)))
  (local cursor (misa.layout.boundary_at_or_before text
                                                  (math.max 0 (math.min (or event.cursor (length text))
                                                                       (length text)))))
  (updated (clear-choice
            (misa.patch (with-text editor text cursor)
                        {:mode :insert :selection_start misa.delete :selection_end misa.delete
                         :attachments (misa.replace attachments)})) nil :restore))

(fn attach [db event]
  (local editor (state db))
  (local attachments {})
  (each [_ item (ipairs (or editor.attachments {}))] (table.insert attachments item))
  (when event.attachment (table.insert attachments event.attachment))
  (updated (misa.patch editor {:attachments (misa.replace attachments)})))

(fn detach [db]
  (local editor (state db))
  (local attachments {})
  (for [index 1 (- (length (or editor.attachments {})) 1)]
    (tset attachments index (. editor.attachments index)))
  (updated (misa.patch editor {:attachments (misa.replace attachments)})))

{:setup (fn [context]
          (local setup-fx [])
          (assert (and (and misa.choice_session misa.layout)
                       misa.layout.previous_boundary)
                  "editor requires choices and layout")
          (local config (or (and (= (type context.config) :table)
                                 context.config.ui)
                            nil))
          (local plain-prompt
                 (and (= (type config) :table) (= config.plain_prompt true)))
          ;; Setup-only named query contributors can precede or follow editor.
          ;; Example: register/service editor_lifecycle.images => [:images/lifecycle].
          ;; Contributors return data flags, never transaction callbacks.
          (table.insert setup-fx
                        {:type :register/sub
                         :value {:id :editor/lifecycle
                                 :inputs (fn []
                                           (local contributors (or misa.editor_lifecycle {}))
                                           (local names (icollect [name (pairs contributors)] name))
                                           (table.sort names)
                                           (icollect [_ name (ipairs names)] (. contributors name)))
                                 :compute (fn [inputs]
                                            (local result {:hold_exit false :block_draft false})
                                            (for [index 1 inputs.n]
                                              (local flags (. inputs index))
                                              (assert (= (type flags) :table) "editor lifecycle query must return flags")
                                              (each [key value (pairs flags)]
                                                (assert (and (or (= key :hold_exit) (= key :block_draft))
                                                             (= (type value) :boolean))
                                                        "invalid editor lifecycle flag")
                                                (when value (tset result key true))))
                                            result)}})
          (table.insert setup-fx
                        {:type :register/service
                         :name :editor_projection
                         :value (fn [db projection-context]
                                  (local editor
                                         (assert db.editor
                                                 "editor state is not initialized"))
                                  (var completions {:rows {}})
                                  (local terminal
                                         (and projection-context
                                              projection-context.terminal))
                                  (local input
                                         (misa.render_component db
                                                                :editor.input
                                                                {:cursor editor.cursor
                                                                 :mode editor.mode
                                                                 :selection_end editor.selection_end
                                                                 :selection_start editor.selection_start
                                                                 :text editor.text}
                                                                {:columns (or (and terminal
                                                                                   terminal.columns)
                                                                              80)}))
                                  (local room
                                         (or (and (and terminal
                                                       misa.inline_choice_room)
                                                  (misa.inline_choice_room db
                                                                           terminal
                                                                           (length input.lines)))
                                             5))
                                  (when (and editor.choice
                                             (not editor.choice_overlay))
                                    (set completions
                                         (completion-layout editor db room
                                                            (or (and terminal
                                                                     terminal.columns)
                                                                80))))
                                  {:busy false
                                   :byte input.cursor.byte
                                   :completions (. (misa.render_component db
                                                                          (or (and completions.columns
                                                                                   :picker)
                                                                              :editor.completions)
                                                                          completions)
                                                   :lines)
                                   :input input.lines
                                   :row input.cursor.row})})

          (local handlers
                 {:app/start (fn [db _ cofx]
                               (local fx {})
                               (when (= (length cofx.argv) 0)
                                 (when (and (not cofx.terminal.interactive) plain-prompt)
                                   (table.insert fx {:lines [{:spans [{:style {:foreground :default}
                                                                     :text "misa> enter a prompt:"}]}]
                                                     :type :view/commit}))
                                 (table.insert fx {:type :terminal/read}))
                               (updated (state db) fx))
                  :agent/status (fn [db event]
                                  (updated (misa.patch (state db) {:busy (not= event.status :ready)}) []))
                  :agent/unavailable (fn [_ event cofx]
                                       {:fx [{:event {:level :error :text event.message :type :transcript/harness}
                                              :type :dispatch}
                                             {:type (if cofx.terminal.interactive :terminal/read :app/quit)}]})
                  :agent/completed (fn [_ event]
                                     {:fx [{:type :dispatch :event {:type :editor/completion-check :exit event.exit}}]})
                  :editor/completion-check (fn [db event cofx]
                                             (local editor db.editor)
                                             (local pending (and cofx.terminal.interactive editor
                                                                 (or (not= editor.text "")
                                                                     (> (length (or editor.attachments {})) 0))))
                                             (local active (and db.agent
                                                                (or (not= db.agent.status :ready)
                                                                    db.agent.startup_prompt)))
                                             {:fx [(if (and event.exit (not active) (not pending)
                                                            (not (. (misa.sub db [:editor/lifecycle]) :hold_exit)))
                                                       {:type :app/quit}
                                                       cofx.terminal.interactive
                                                       {:event {:type :ui/redraw} :type :dispatch}
                                                       {:type :terminal/read})]})
                  :editor/restore restore
                  :editor/attach attach
                  :editor/detach detach
                  :editor/steer (fn [db _ cofx]
                                  (local editor (state db))
                                  (if (. (misa.sub db [:editor/lifecycle]) :block_draft)
                                      (updated editor)
                                      (let [result (dispatch-input (misa.patch (emptied editor)
                                                                              {:mode :insert :selection_start misa.delete
                                                                               :selection_end misa.delete})
                                                                   {:attachments editor.attachments :prompt editor.text
                                                                    :type :queue/steer} cofx)]
                                        (values result :steer))))
                  :ui/redraw (fn [] {:fx [{:type :terminal/read}]})
                  :editor/choice-selected selected
                  :terminal/input terminal-input})
          (each [name handler (pairs handlers)]
            (table.insert setup-fx
                          {:type :register/event : name
                           :handler (fn [db event cofx]
                                      (local (result reason) (handler db event cofx))
                                      (accounted db result (or reason :preserve) cofx))}))
          (table.insert setup-fx
                        {:type :register/setup-effect :name :register/editor-edit
                         :handler (fn [effect]
                                    (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                 (= (type effect.value) :function) (not (. edits effect.id)))
                                            "invalid or duplicate editor edit")
                                    (tset edits effect.id effect.value))})
          {:fx setup-fx})}
