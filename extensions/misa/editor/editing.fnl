;; Modal editing policy over the editor's text/cursor model. Motions operate on
;; grapheme boundaries; neither terminal decoding nor the view knows Vim.

(fn line-start [text at]
  "Return the start position of the containing line."
  (- (or (: (text:sub 1 at) :match ".*\n()") 1) 1))

(fn line-end [text at]
  "Return the end position of the containing line."
  (- (or (text:find "\n" (+ at 1) true) (+ (length text) 1)) 1))

(fn prev [text at]
  "Return the preceding text boundary."
  (misa.layout.previous-boundary text at))

(fn next-at [text at]
  "Return the following text boundary."
  (misa.layout.next-boundary text at))

(fn class [text at]
  (let [c (text:sub (+ at 1) (next-at text at))]
    (if (= c "")
        :end
        (or (and (c:match "%s") :space)
            (and (or (c:match "[%w_]") (>= (or (c:byte) 0) 128)) :word)
            :punctuation))))

(fn vertical [editor direction]
  "Move the cursor vertically while preserving its column."
  (let [text editor.text
        at editor.cursor
        start (line-start text at)
        column (misa.layout.width (text:sub (+ start 1) at))
        target (if (= direction :down) (+ (line-end text at) 1)
                   (> start 0) (line-start text (- start 1))
                   nil)]
    (if (or (= target nil) (> target (length text))) at
        (let [finish (line-end text target)
              (_ bytes) (misa.layout.clip (text:sub (+ target 1) finish) column)]
          (+ target bytes)))))

(fn word-end [editor]
  "Return the end of the current or following word."
  (let [text editor.text]
    (var at (next-at text editor.cursor))
    (while (and (< at (length text)) (= (class text at) :space))
      (set at (next-at text at)))
    (let [group (class text at)]
      (while (and (< (next-at text at) (length text))
                  (= (class text (next-at text at)) group))
        (set at (next-at text at)))
      at)))

(fn word-previous [editor]
  "Return the start of the previous word."
  (let [text editor.text]
    (var at (prev text editor.cursor))
    (while (and (> at 0) (= (class text at) :space))
      (set at (prev text at)))
    (let [group (class text at)]
      (while (and (> at 0) (= (class text (prev text at)) group))
        (set at (prev text at)))
      at)))

(fn word-next [editor]
  "Return the start of the next word."
  (let [text editor.text]
    (var at editor.cursor)
    (let [group (class text at)]
      (while (and (< at (length text)) (= (class text at) group))
        (set at (next-at text at)))
      (while (and (< at (length text)) (= (class text at) :space))
        (set at (next-at text at)))
      at)))

(fn selection [editor state]
  (if (= state.anchor nil) nil
      (let [first (math.min state.anchor editor.cursor)
            last (math.max state.anchor editor.cursor)]
        (if state.linewise
            (values (line-start editor.text first)
                    (math.min (length editor.text)
                              (+ (line-end editor.text last) 1)))
            (values first (next-at editor.text last))))))

(fn reset-navigation []
  {:anchor misa.delete :operator misa.delete :insert_group misa.delete})

(fn appended [entries value]
  (let [result {}
        source (or entries {})]
    (for [index (math.max 1 (- (length source) 62)) (length source)]
      (table.insert result (. source index)))
    (table.insert result value)
    result))

(fn remember [state text cursor]
  (misa.patch state {:undo (misa.replace (appended state.undo {: text : cursor}))
                     :redo (misa.replace {})}))

;; The editor supplies the operation's meaning at the transition boundary.
;; This policy never observes unrelated transactions or retains editor snapshots.
(fn transition [state previous editor reason]
  (if (or (= reason :restore) (= reason :steer))
      (misa.patch state {:anchor misa.delete
                         :operator misa.delete
                         :insert_group misa.delete
                         :undo (misa.replace {})
                         :redo (misa.replace {})})
      (= reason :discard)
      (misa.patch state {:undo (misa.replace {})
                         :redo (misa.replace {})
                         :insert_group misa.delete})
      (and (not= previous.text editor.text) (= reason :insert)
           (not state.insert_group))
      (misa.patch (remember state previous.text previous.cursor)
                  {:insert_group true})
      (and (not= previous.text editor.text) (= reason :edit))
      (remember state previous.text previous.cursor)
      state))

(fn insert [editor _ event]
  "Apply an insert-mode action to the editor."
  (let [action event.action
        places {:append next-at
                :insert_start line-start
                :append_end line-end
                :open_below line-end
                :open_above line-start}
        position (. places action)
        at (if position (position editor.text editor.cursor) editor.cursor)
        open (or (= action :open_above) (= action :open_below))]
    {:editor {:text (if open
                        (.. (editor.text:sub 1 at) "\n"
                            (editor.text:sub (+ at 1)))
                        editor.text)
              :cursor (if (= action :open_below) (+ at 1) at)
              :mode :insert}
     :editing (reset-navigation)}))

(fn visual [editor _ event]
  "Toggle structural range selection."
  {:editor {:mode :visual}
   :editing {:anchor editor.cursor :linewise (= event.action :visual_line)}})

(fn undo [editor state event]
  "Move between the editor undo and redo histories."
  (let [from (if (= event.action :undo) :undo :redo)
        to (if (= event.action :undo) :redo :undo)
        source (or (. state from) {})
        value (. source (length source))
        rest {}]
    (for [index 1 (- (length source) 1)]
      (tset rest index (. source index)))
    (let [patch (reset-navigation)]
      (tset patch from (misa.replace rest))
      (tset patch to
            (misa.replace (if value
                              (appended (. state to)
                                        {:text editor.text
                                         :cursor editor.cursor})
                              (or (. state to) {}))))
      {:editor {:mode :normal
                :text (and value value.text)
                :cursor (and value value.cursor)}
       :editing patch})))

(fn paste [editor _ _event db]
  "Insert clipboard contents using characterwise or linewise placement."
  (let [register (or db.clipboard {})]
    (var text (or register.text ""))
    (var at (next-at editor.text editor.cursor))
    (when (and register.linewise (not= text ""))
      (let [finish (line-end editor.text editor.cursor)]
        (if (< finish (length editor.text))
            (do
              (set at (+ finish 1))
              (when (not= (text:sub (- 1)) "\n")
                (set text (.. text "\n"))))
            (do
              (set at finish)
              (set text (.. "\n" (text:gsub "\n$" "")))))))
    {:editor {:text (.. (editor.text:sub 1 at) text (editor.text:sub (+ at 1)))
              :cursor (+ at (length text))}}))

(fn submit []
  "Return to insert mode and submit the editor input."
  {:editor {:mode :insert}
   :editing {:anchor misa.delete}
   :fx [{:event {:type :terminal/input :kind :enter} :type :dispatch}]})

(fn normal [editor]
  "Enter normal editing mode and reset motion state."
  {:editor {:mode :normal
            :cursor (if (and (= (or editor.mode :insert) :insert)
                             (> editor.cursor
                                (line-start editor.text editor.cursor)))
                        (prev editor.text editor.cursor)
                        editor.cursor)}
   :editing (reset-navigation)})

(local operators {:delete true :change true :yank true})

(fn operate [editor state event]
  (let [action event.action
        movement (. (misa.catalog :editing-motions) action)
        target (and movement (movement editor))]
    (var (first last) (selection editor state))
    (var operator state.operator)
    (var linewise (and (not= first nil) (= state.linewise true)))
    (let [editor-patch {}
          state-patch {}]
      (if target
          (if operator
              (do
                (set (first last)
                     (values (math.min editor.cursor target)
                             (math.max editor.cursor target)))
                (when (= action :word_end)
                  (set last (next-at editor.text last))))
              (tset editor-patch :cursor target))
          (= action :delete_character)
          (set (first last operator)
               (values editor.cursor (next-at editor.text editor.cursor)
                       :delete))
          (. operators action)
          (if first (set operator action) (= operator action)
              (do
                (set linewise true)
                (set (first last)
                     (values (line-start editor.text editor.cursor)
                             (math.min (length editor.text)
                                       (+ (line-end editor.text editor.cursor)
                                          1)))))
              (tset state-patch :operator action))
          (tset state-patch :operator misa.delete))
      (if (and first (or operator (. operators action)))
          (let [op (or operator action)
                text (editor.text:sub (+ first 1) last)]
            {:editor {:text (when (not= op :yank)
                              (.. (editor.text:sub 1 first)
                                  (editor.text:sub (+ last 1))))
                      :cursor (when (not= op :yank) first)
                      :mode (if (= op :change) :insert :normal)}
             :editing (reset-navigation)
             :fx [{:event {: linewise : text :type :clipboard/copy}
                   :type :dispatch}]})
          {:editor editor-patch :editing state-patch}))))

(fn action [db event]
  "Apply a modal editing action to the current draft."
  (if (not db.editor) {:fx [{:type :terminal/read}]}
      (let [previous db.editor
            editor (misa.patch previous
                               {:choice misa.delete
                                :choice_kind misa.delete
                                :choice_command misa.delete
                                :choice_overlay misa.delete})
            state (or db.editing {})
            handler (or (. (misa.catalog :editing-actions) event.action)
                        operate)
            result (handler editor state event db)
            next-editor (misa.patch editor (or result.editor {}))
            next-state (transition (misa.patch state (or result.editing {}))
                                   previous next-editor
                                   (if (or (= event.action :undo)
                                           (= event.action :redo))
                                       event.action
                                       :edit))
            (first last) (selection next-editor next-state)
            fx [{:type :terminal/read}]]
        (each [_ effect (ipairs (or result.fx {}))]
          (table.insert fx effect))
        {:patch {:editor (misa.replace (misa.patch next-editor
                                                   {:selection_start (misa.replace first)
                                                    :selection_end (misa.replace last)}))
                 :editing (misa.replace next-state)}
         : fx})))

(local interrupt-inputs {:ctrl_c true :ctrl_d true :eof true})

(fn route-input [db event cofx enabled]
  "Route terminal input according to the selected editing mode."
  (when (and enabled cofx.terminal.interactive (not db.dialog) (not db.picker)
             (not db.selection) db.editor)
    (if (= (or db.editor.mode :insert) :insert)
        (when (= event.kind :escape) {:action :normal :type :editing/action})
        (. interrupt-inputs event.kind)
        {:type :editing/interrupt :input event}
        {:action (or (misa.keybindings.action :editor.normal event) :ignore)
         :type :editing/action})))

(fn available? [db]
  "Return whether modal editing is available."
  (and db.editor (not db.selection)))

(fn on-editing-interrupt [_ event]
  "Return modal editing to insert mode after an interrupt."
  {:patch {:editor {:mode :insert
                    :selection_start misa.delete
                    :selection_end misa.delete}
           :editing {:anchor misa.delete}}
   :fx [{:type :dispatch :event event.input}]})

(fn editing-actions [_ handler]
  "Validate an editing action."
  (assert (= (type handler) :function) "action must be a function"))

(fn editing-motions [_ handler]
  "Validate an editing motion."
  (assert (= (type handler) :function) "motion must be a function"))

(fn enabled? [config]
  "Return whether modal editing is enabled, validating the selected mode."
  (let [mode (or (. (or config.editing {}) :mode) :vim)]
    (assert (or (= mode :vim) (= mode :plain))
            "editing.mode must be vim or plain")
    (= mode :vim)))

(fn editor-transition [enabled state previous editor reason interactive]
  "Reconcile editing history after an editor transition."
  (if (or (= reason :restore) (= reason :steer) (and enabled interactive))
      (transition state previous editor reason)
      state))

{:action action
 :available? available?
 :editing-actions editing-actions
 :editing-motions editing-motions
 :editor-transition editor-transition
 :enabled? enabled?
 :on-editing-interrupt on-editing-interrupt
 :route-input route-input
 :submit submit
 :next-at next-at
 :line-start line-start
 :insert insert
 :visual visual
 :normal normal
 :word-previous word-previous
 :paste paste
 :prev prev
 :word-next word-next
 :undo undo
 :vertical vertical
 :word-end word-end
 :line-end line-end}
