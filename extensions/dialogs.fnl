;; Generic correlated in-UI interactions. This module owns lifecycle and input;

;; producers supply semantic data and never select a visual implementation.

(fn copy-actions [actions]
  (let [result {}]
    (each [index action (ipairs (or actions {}))]
      (assert (and (and (= (type action) :table) (= (type action.id) :string))
                   (not= action.id "")) "invalid dialog action")
      (tset result index
            {:id action.id
             :label (or action.label action.id)
             :primary (= action.primary true)}))
    result))

(fn finish [state action cancelled]
  [{:event {: action
            :cancelled (= cancelled true)
            :correlation state.correlation
            :id state.id
            :protected state.protected
            :type state.completion
            :value (or state.input "")}
    :type :dispatch}])

(fn first-action [actions]
  (each [index action (ipairs actions)]
    (when action.primary (lua "return index")))
  1)

{:setup (fn []
          (set misa.dialogs true)
          (misa.reg_event :dialog/open
                          (fn [db event]
                            (assert (not db.dialog) "a dialog is already open")
                            (assert (and (and (and (= (type event.id) :string)
                                                   (not= event.id ""))
                                              (= (type event.correlation)
                                                 :string))
                                         (not= event.correlation ""))
                                    "invalid dialog identity")
                            (assert (and (= (type event.completion) :string)
                                         (not= event.completion ""))
                                    "invalid dialog completion")
                            (local kind (or event.kind :modal))
                            (assert (or (or (= kind :modal) (= kind :progress))
                                        (= kind :alert))
                                    "invalid dialog kind")
                            (set db.dialog
                                 {:actions (copy-actions event.actions)
                                  :cancellable (= event.cancellable true)
                                  :code event.code
                                  :completion event.completion
                                  :correlation event.correlation
                                  :hints (or event.hints {})
                                  :id event.id
                                  :input (or (and event.protected "")
                                             (or event.initial ""))
                                  :input_enabled (= event.input true)
                                  :input_length 0
                                  : kind
                                  :message (or event.message "")
                                  :progress event.progress
                                  :protected (= event.protected true)
                                  :title (or event.title "")
                                  :url event.url})
                            (set db.dialog.selected_action
                                 (first-action db.dialog.actions))
                            (local fx [{:type :terminal/read}])
                            (when event.protected
                              (tset fx (+ (length fx) 1)
                                    {:completion :dialog/protected-input
                                     :correlation event.correlation
                                     :id event.id
                                     :type :input/protected}))
                            {: db : fx}))
          (misa.reg_event :dialog/update
                          (fn [db event]
                            (local state db.dialog)
                            (if (or (or (not state) (not= state.id event.id))
                                    (not= state.correlation event.correlation))
                                nil
                                (do
                                  (each [_ name (ipairs [:kind
                                                         :title
                                                         :message
                                                         :url
                                                         :code
                                                         :progress
                                                         :cancellable])]
                                    (when (not= (. event name) nil)
                                      (tset state name (. event name))))
                                  (when (not= event.hints nil)
                                    (set state.hints event.hints))
                                  (when (not= event.actions nil)
                                    (set state.actions
                                         (copy-actions event.actions))
                                    (set state.selected_action
                                         (first-action state.actions)))
                                  (when (not= event.input nil)
                                    (set state.input_enabled
                                         (= event.input true)))
                                  {: db :fx [{:type :terminal/read}]}))))
          (misa.reg_event :dialog/close
                          (fn [db event]
                            (local state db.dialog)
                            (if (or (or (not state) (not= state.id event.id))
                                    (and event.correlation
                                         (not= state.correlation
                                               event.correlation)))
                                nil (do
                                     (set db.dialog nil)
                                     {: db}))))
          (misa.reg_interceptor {:before (fn [tx]
                                           (when (and (= tx.event.type
                                                         :terminal/input)
                                                      tx.db.dialog)
                                             (set tx.event
                                                  {:kind tx.event.kind
                                                   :text tx.event.text
                                                   :type :dialog/input}))
                                           tx)
                                 :id :dialogs/input})
          (misa.reg_event :dialog/protected-input
                          (fn [db event]
                            (local state db.dialog)
                            (if (or (or (or (not state) (not state.protected))
                                        (not= state.id event.id))
                                    (not= state.correlation event.correlation))
                                nil
                                (do
                                  (set state.input_length (or event.length 0))
                                  (set state.input_error
                                       (or (and event.too_long
                                                "Input is too long; shorten it before submitting.")
                                           nil))
                                  (if (or event.submitted event.cancelled)
                                      (do
                                        (set db.dialog nil)
                                        {: db
                                         :fx (finish state
                                                     (or (and event.cancelled
                                                              :cancel)
                                                         :submit)
                                                     event.cancelled)})
                                      {: db :fx [{:type :terminal/read}]})))))
          (misa.reg_event :dialog/input
                          (fn [db event]
                            (local state (assert db.dialog))
                            (if state.protected
                                {: db :fx [{:type :terminal/read}]}
                                (if (and (or (or (or (= event.kind :escape)
                                                     (= event.kind :ctrl_c))
                                                 (= event.kind :ctrl_d))
                                             (= event.kind :eof))
                                         state.cancellable)
                                    (do
                                      (set db.dialog nil)
                                      {: db :fx (finish state :cancel true)})
                                    (if (and (> (length state.actions) 1)
                                             (or (or (= event.kind :tab)
                                                     (= event.kind :arrow_right))
                                                 (= event.kind :arrow_left)))
                                        (do
                                          (local direction
                                                 (or (and (= event.kind
                                                             :arrow_left)
                                                          (- 1))
                                                     1))
                                          (set state.selected_action
                                               (+ (% (+ (- (or state.selected_action
                                                               1)
                                                           1)
                                                        direction)
                                                     (length state.actions))
                                                  1))
                                          {: db :fx [{:type :terminal/read}]})
                                        (do
                                          (if state.input_enabled
                                              (if (= event.kind :text)
                                                  (set state.input
                                                       (.. state.input
                                                           (or event.text "")))
                                                  (= event.kind :backspace)
                                                  (do
                                                    (var last
                                                         (- (length state.input)
                                                            1))
                                                    (while (and (and (> last 0)
                                                                     (>= (state.input:byte (+ last
                                                                                              1))
                                                                         128))
                                                                (< (state.input:byte (+ last
                                                                                        1))
                                                                   192))
                                                      (set last (- last 1)))
                                                    (set state.input
                                                         (state.input:sub 1
                                                                          last)))
                                                  (= event.kind :enter)
                                                  (do
                                                    (local selected
                                                           (. state.actions
                                                              (or state.selected_action
                                                                  1)))
                                                    (local action
                                                           (or (and selected
                                                                    selected.id)
                                                               :submit))
                                                    (set db.dialog nil)
                                                    (let [___antifnl_rtn_1___ {: db
                                                                               :fx (finish state
                                                                                           action
                                                                                           false)}]
                                                      (lua "return ___antifnl_rtn_1___"))))
                                              (and (= event.kind :enter)
                                                   (> (length state.actions) 0))
                                              (do
                                                (local action
                                                       (. state.actions
                                                          (or state.selected_action
                                                              1)
                                                          :id))
                                                (set db.dialog nil)
                                                (let [___antifnl_rtn_1___ {: db
                                                                           :fx (finish state
                                                                                       action
                                                                                       false)}]
                                                  (lua "return ___antifnl_rtn_1___"))))
                                          {: db :fx [{:type :terminal/read}]}))))))
          nil)}

