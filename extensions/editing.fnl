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


(fn vertical [editor direction]
  (local (text at) (values editor.text editor.cursor))
  (local start (line-start text at))
  (local column (misa.layout.width (text:sub (+ start 1) at)))
  (local target (if (= direction :down) (+ (line-end text at) 1)
                    (> start 0) (line-start text (- start 1)) nil))
  (if (or (= target nil) (> target (length text))) at
      (let [finish (line-end text target)
            (_ bytes) (misa.layout.clip (text:sub (+ target 1) finish) column)]
        (+ target bytes))))

(local motions
       {:left (fn [editor] (prev editor.text editor.cursor))
        :right (fn [editor] (next-at editor.text editor.cursor))
        :line_start (fn [editor] (line-start editor.text editor.cursor))
        :line_end (fn [editor] (line-end editor.text editor.cursor))
        :down (fn [editor] (vertical editor :down))
        :up (fn [editor] (vertical editor :up))
        :first (fn [] 0)
        :last (fn [editor] (length editor.text))
        :word_next (fn [editor]
                     (local text editor.text)
                     (var at editor.cursor)
                     (local group (class text at))
                     (while (and (< at (length text)) (= (class text at) group))
                       (set at (next-at text at)))
                     (while (and (< at (length text)) (= (class text at) :space))
                       (set at (next-at text at)))
                     at)
        :word_previous (fn [editor]
                         (local text editor.text)
                         (var at (prev text editor.cursor))
                         (while (and (> at 0) (= (class text at) :space))
                           (set at (prev text at)))
                         (local group (class text at))
                         (while (and (> at 0) (= (class text (prev text at)) group))
                           (set at (prev text at)))
                         at)
        :word_end (fn [editor]
                    (local text editor.text)
                    (var at (next-at text editor.cursor))
                    (while (and (< at (length text)) (= (class text at) :space))
                      (set at (next-at text at)))
                    (local group (class text at))
                    (while (and (< (next-at text at) (length text))
                                (= (class text (next-at text at)) group))
                      (set at (next-at text at)))
                    at)})

(fn selection [editor state]
  (if (= state.anchor nil) nil
      (let [(first last) (values (math.min state.anchor editor.cursor)
                                 (math.max state.anchor editor.cursor))]
        (if state.linewise
            (values (line-start editor.text first)
                    (math.min (length editor.text)
                              (+ (line-end editor.text last) 1)))
            (values first (next-at editor.text last))))))


(fn reset-navigation []
  {:anchor misa.delete :operator misa.delete :insert_group misa.delete})

(fn appended [entries value]
  (local result {})
  (local source (or entries {}))
  (for [index (math.max 1 (- (length source) 62)) (length source)]
    (table.insert result (. source index)))
  (table.insert result value)
  result)

(fn remember [state text cursor]
  (misa.patch state {:undo (misa.replace (appended state.undo {: text : cursor}))
                     :redo (misa.replace {})}))

;; The editor supplies the operation's meaning at the transition boundary.
;; This policy never observes unrelated transactions or retains editor snapshots.
(fn transition [state previous editor reason]
  (if (or (= reason :restore) (= reason :steer))
      (misa.patch state {:anchor misa.delete :operator misa.delete :insert_group misa.delete
                         :undo (misa.replace {}) :redo (misa.replace {})})
      (= reason :discard)
      (misa.patch state {:undo (misa.replace {}) :redo (misa.replace {})
                         :insert_group misa.delete})
      (and (not= previous.text editor.text) (= reason :insert) (not state.insert_group))
      (misa.patch (remember state previous.text previous.cursor) {:insert_group true})
      (and (not= previous.text editor.text) (= reason :edit))
      (remember state previous.text previous.cursor)
      state))

(fn insert [editor _ event]
  (local action event.action)
  (local places {:append next-at :insert_start line-start :append_end line-end
                 :open_below line-end :open_above line-start})
  (local position (. places action))
  (local at (if position (position editor.text editor.cursor) editor.cursor))
  (local open (or (= action :open_above) (= action :open_below)))
  {:editor {:text (if open (.. (editor.text:sub 1 at) "\n" (editor.text:sub (+ at 1))) editor.text)
            :cursor (if (= action :open_below) (+ at 1) at) :mode :insert}
   :editing (reset-navigation)})

(fn visual [editor _ event]
  {:editor {:mode :visual}
   :editing {:anchor editor.cursor :linewise (= event.action :visual_line)}})

(fn undo [editor state event]
  (local from (if (= event.action :undo) :undo :redo))
  (local to (if (= event.action :undo) :redo :undo))
  (local source (or (. state from) {}))
  (local value (. source (length source)))
  (local rest {})
  (for [index 1 (- (length source) 1)] (tset rest index (. source index)))
  (local patch (reset-navigation))
  (tset patch from (misa.replace rest))
  (tset patch to (misa.replace (if value
                                 (appended (. state to) {:text editor.text :cursor editor.cursor})
                                 (or (. state to) {}))))
  {:editor {:mode :normal :text (and value value.text) :cursor (and value value.cursor)}
   :editing patch})

(fn paste [editor _ _event db]
  (local register (or db.clipboard {}))
  (var text (or register.text ""))
  (var at (next-at editor.text editor.cursor))
  (when (and register.linewise (not= text ""))
    (local finish (line-end editor.text editor.cursor))
    (if (< finish (length editor.text))
        (do
          (set at (+ finish 1))
          (when (not= (text:sub (- 1)) "\n") (set text (.. text "\n"))))
        (do
          (set at finish)
          (set text (.. "\n" (text:gsub "\n$" ""))))))
  {:editor {:text (.. (editor.text:sub 1 at) text (editor.text:sub (+ at 1)))
            :cursor (+ at (length text))}})

(local actions
       {:normal (fn [editor]
                  {:editor {:mode :normal
                            :cursor (if (and (= (or editor.mode :insert) :insert)
                                             (> editor.cursor (line-start editor.text editor.cursor)))
                                        (prev editor.text editor.cursor) editor.cursor)}
                   :editing (reset-navigation)})
        :submit (fn []
                  {:editor {:mode :insert} :editing {:anchor misa.delete}
                   :fx [{:event {:type :terminal/input :kind :enter} :type :dispatch}]})
        :insert insert :append insert :insert_start insert :append_end insert
        :open_below insert :open_above insert
        :visual visual :visual_line visual
        :undo undo :redo undo :paste paste})

(local operators {:delete true :change true :yank true})

(fn operate [editor state event]
  (local action event.action)
  (local movement (. motions action))
  (local target (and movement (movement editor)))
  (var (first last) (selection editor state))
  (var operator state.operator)
  (var linewise (and (not= first nil) (= state.linewise true)))
  (local (editor-patch state-patch) (values {} {}))
  (if target
      (if operator
          (do
            (set (first last) (values (math.min editor.cursor target) (math.max editor.cursor target)))
            (when (= action :word_end) (set last (next-at editor.text last))))
          (tset editor-patch :cursor target))
      (= action :delete_character)
      (set (first last operator) (values editor.cursor (next-at editor.text editor.cursor) :delete))
      (. operators action)
      (if first (set operator action)
          (= operator action)
          (do
            (set linewise true)
            (set (first last) (values (line-start editor.text editor.cursor)
                                      (math.min (length editor.text) (+ (line-end editor.text editor.cursor) 1)))))
          (tset state-patch :operator action))
      (tset state-patch :operator misa.delete))
  (if (and first (or operator (. operators action)))
      (let [op (or operator action)
            text (editor.text:sub (+ first 1) last)]
        {:editor {:text (when (not= op :yank)
                          (.. (editor.text:sub 1 first) (editor.text:sub (+ last 1))))
                  :cursor (when (not= op :yank) first)
                  :mode (if (= op :change) :insert :normal)}
         :editing (reset-navigation)
         :fx [{:event {: linewise : text :type :clipboard/copy} :type :dispatch}]})
      {:editor editor-patch :editing state-patch}))

(fn action [db event]
  (if (not db.editor) {:fx [{:type :terminal/read}]}
      (let [previous db.editor
            editor (misa.patch previous {:choice misa.delete :choice_kind misa.delete
                                         :choice_command misa.delete :choice_overlay misa.delete})
            state (or db.editing {})
            handler (or (. actions event.action) operate)
            result (handler editor state event db)
            next-editor (misa.patch editor (or result.editor {}))]
        (local next-state (transition (misa.patch state (or result.editing {}))
                                      previous next-editor
                                      (if (or (= event.action :undo) (= event.action :redo))
                                          event.action :edit)))
        (local (first last) (selection next-editor next-state))
        (local fx [{:type :terminal/read}])
        (each [_ effect (ipairs (or result.fx {}))] (table.insert fx effect))
        {:patch {:editor (misa.replace (misa.patch next-editor
                                                  {:selection_start (misa.replace first)
                                                   :selection_end (misa.replace last)}))
                 :editing (misa.replace next-state)}
         : fx})))

(local interrupt-inputs {:ctrl_c true :ctrl_d true :eof true})

(fn route-input [db event cofx enabled]
  (when (and enabled cofx.terminal.interactive (not db.dialog) (not db.picker)
             (not db.selection) db.editor)
    (if (= (or db.editor.mode :insert) :insert)
        (when (= event.kind :escape) {:action :normal :type :editing/action})
        (. interrupt-inputs event.kind)
        {:type :editing/interrupt :input event}
        {:action (or (misa.keybinding_action :editor.normal event) :ignore)
         :type :editing/action})))

{:setup (fn [context]
          (local setup-fx [])
          (local mode (or (. (or context.config.editing {}) :mode) :vim))
          (assert (or (= mode :vim) (= mode :plain)) "editing.mode must be vim or plain")
          (local enabled (= mode :vim))
          (table.insert setup-fx
                        {:type :register/service :name :editing_transition
                         :value (fn [state previous editor reason interactive]
                                  (if (or (= reason :restore) (= reason :steer)
                                          (and enabled interactive))
                                      (transition state previous editor reason) state))})
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
            (table.insert setup-fx
                          {:type :register/keybinding
                           :value {: action
                                   :context :editor.normal
                                   :default keys}})
            (table.insert setup-fx
                          {:type :register/action
                           :value {: available
                                   :binding {: action :context :editor.normal}
                                   :event {: action :type :editing/action}
                                   :id (.. :editor. action)
                                   :label (.. "Editor: " (action:gsub "_" " "))}}))

          (table.insert setup-fx
                        {:type :register/event-route
                         :value {:id :editing/input :event :terminal/input :priority 200
                                 :context [:db/path]
                                 :resolve (fn [db event cofx] (route-input db event cofx enabled))}})
          (table.insert setup-fx
                        {:type :register/event :name :editing/interrupt
                         :handler (fn [_ event]
                                    {:patch {:editor {:mode :insert :selection_start misa.delete
                                                      :selection_end misa.delete}
                                             :editing {:anchor misa.delete}}
                                     :fx [{:type :dispatch :event event.input}]})})
          (table.insert setup-fx {:type :register/event :name :editing/action :handler action})
          (each [name registry (pairs {:register/editing-motion motions :register/editing-action actions})]
            (table.insert setup-fx
                          {:type :register/setup-effect : name
                           :handler (fn [effect]
                                      (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                   (= (type effect.value) :function) (not (. registry effect.id)))
                                              "invalid or duplicate editing transition")
                                      (tset registry effect.id effect.value))}))
          {:fx setup-fx})}
