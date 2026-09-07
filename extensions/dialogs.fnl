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


(fn updated [state patch fx]
  {:patch {:dialog (misa.replace (misa.patch state patch))}
   :fx (or fx [{:type :terminal/read}])})

(fn completed [state action cancelled]
  {:patch {:dialog misa.delete} :fx (finish state action cancelled)})

(fn correlated [state event]
  (and state (= state.id event.id) (= state.correlation event.correlation)))

(fn open [db event]
  (assert (not db.dialog) "a dialog is already open")
  (assert (and (= (type event.id) :string) (not= event.id "")
               (= (type event.correlation) :string) (not= event.correlation ""))
          "invalid dialog identity")
  (assert (and (= (type event.completion) :string) (not= event.completion ""))
          "invalid dialog completion")
  (local kind (or event.kind :modal))
  (assert (or (= kind :modal) (= kind :progress) (= kind :alert)) "invalid dialog kind")
  (local actions (copy-actions event.actions))
  (local state {: actions :selected_action (first-action actions)
                :cancellable (= event.cancellable true) :code event.code
                :completion event.completion :correlation event.correlation
                :hints (or event.hints {}) :id event.id
                :input (if event.protected "" (or event.initial ""))
                :input_enabled (= event.input true) :input_length 0
                : kind :message (or event.message "") :progress event.progress :sections event.sections
                :protected (= event.protected true) :title (or event.title "") :url event.url})
  (updated state {}
           (if event.protected
               [{:type :terminal/read}
                {:completion :dialog/protected-input :correlation event.correlation
                 :id event.id :type :input/protected}]
               [{:type :terminal/read}])))

(fn update [db event]
  (local state db.dialog)
  (when (correlated state event)
    (local patch {})
    (each [_ name (ipairs [:kind :title :message :url :code :progress :cancellable :hints :sections])]
      (when (not= (. event name) nil) (tset patch name (misa.replace (. event name)))))
    (when (not= event.actions nil)
      (local actions (copy-actions event.actions))
      (tset patch :actions (misa.replace actions))
      (tset patch :selected_action (first-action actions)))
    (when (not= event.input nil) (tset patch :input_enabled (= event.input true)))
    (updated state patch)))

(fn close [db event]
  (when (and db.dialog (= db.dialog.id event.id)
             (or (not event.correlation) (= db.dialog.correlation event.correlation)))
    {:patch {:dialog misa.delete}}))

(fn protected-input [db event]
  (local state db.dialog)
  (when (and (correlated state event) state.protected)
    (if (or event.submitted event.cancelled)
        (completed state (if event.cancelled :cancel :submit) event.cancelled)
        (updated state
                 {:input_length (or event.length 0)
                  :input_error (misa.replace
                                (when event.too_long "Input is too long; shorten it before submitting."))}))))

(fn cancel [state]
  (if state.cancellable (completed state :cancel true) {:fx [{:type :terminal/read}]}))

(fn navigate [state event]
  (if (> (length state.actions) 1)
      (updated state {:selected_action (+ (% (+ (- (or state.selected_action 1) 1)
                                                 (if (= event.kind :arrow_left) (- 1) 1))
                                              (length state.actions)) 1)})
      {:fx [{:type :terminal/read}]}))

(local inputs
       {:escape cancel :ctrl_c cancel :ctrl_d cancel :eof cancel
        :tab navigate :arrow_left navigate :arrow_right navigate
        :text (fn [state event]
                (if state.input_enabled
                    (updated state {:input (.. state.input (or event.text ""))})
                    {:fx [{:type :terminal/read}]}))
        :backspace (fn [state]
                     (if state.input_enabled
                         (let [text state.input]
                           (var at (length text))
                           (while (and (> at 0) (>= (text:byte at) 128) (< (text:byte at) 192))
                             (set at (- at 1)))
                           (updated state {:input (text:sub 1 (math.max 0 (- at 1)))}))
                         {:fx [{:type :terminal/read}]}))
        :enter (fn [state]
                 (if (or state.input_enabled (> (length state.actions) 0))
                     (let [selected (. state.actions (or state.selected_action 1))]
                       (completed state (or (and selected selected.id) :submit) false))
                     {:fx [{:type :terminal/read}]}))})

(fn input [db event]
  (local state (assert db.dialog))
  (local handler (and (not state.protected) (. inputs event.kind)))
  (if handler (handler state event) {:fx [{:type :terminal/read}]}))

{:setup (fn []
          (local fx [{:type :register/service :name :dialogs :value true}
                     {:type :register/interceptor
                      :value {:id :dialogs/input
                              :before (fn [tx]
                                        (if (and (= tx.event.type :terminal/input) tx.db.dialog)
                                          (misa.patch tx {:event (misa.replace {:kind tx.event.kind :text tx.event.text :type :dialog/input})})
                                          tx))}}
                     {:type :register/setup-effect :name :register/dialog-input
                      :handler (fn [effect]
                                 (assert (and (= (type effect.id) :string) (not= effect.id "")
                                              (= (type effect.value) :function) (not (. inputs effect.id)))
                                         "invalid or duplicate dialog input")
                                 (tset inputs effect.id effect.value))}])
          (each [name handler (pairs {:dialog/open open :dialog/update update :dialog/close close
                                      :dialog/protected-input protected-input :dialog/input input})]
            (table.insert fx {:type :register/event : name : handler}))
          {: fx})}
