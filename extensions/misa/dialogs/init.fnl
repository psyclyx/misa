(local definitions (require :misa.definitions))

;; Generic correlated interactions own input, buttons, and confirmation lifecycle.
(fn copy-actions [actions]
  (let [result []
        ids {}]
    (var primary false)
    (each [_ action (ipairs (or actions []))]
      (assert (and (= (type action) :table) (= (type action.id) :string)
                   (not= action.id "") (not= action.id :dialog-close)
                   (not (. ids action.id)))
              "invalid, reserved, or duplicate dialog action")
      (assert (not (and primary action.primary))
              "multiple primary dialog actions")
      (set primary (or primary action.primary))
      (tset ids action.id true)
      (table.insert result (misa.snapshot action)))
    result))

(fn token [state action]
  "Return a dialog-scoped token for an action."
  (.. "dialog.action/" (length state.id) ":" state.id
      (length state.correlation) ":" state.correlation action.id))

(fn buttons [state]
  "Build button descriptors for the current dialog."
  (if state.confirmation
      [{:id :confirm
        :label (or state.confirmation.label "Confirm")
        :key :enter}
       {:id :cancel-confirmation :label "Cancel" :key :escape}]
      (let [result (misa.snapshot (or state.actions []))]
        (when state.cancellable
          (table.insert result {:id :dialog-close :label "Close" :key :escape}))
        result)))

(fn updated [state patch fx]
  {:patch {:dialog (misa.replace (misa.patch state patch))}
   :fx (or fx [{:type :terminal/read}])})

(fn completed [state action cancelled persistent]
  {:patch {:dialog (if persistent
                       (misa.replace (misa.patch state
                                                 {:confirmation misa.delete}))
                       misa.delete)}
   :fx [{:type :dispatch
         :event {:action action
                 :cancelled (= cancelled true)
                 :correlation state.correlation
                 :id state.id
                 :protected state.protected
                 :type state.completion
                 :value (or state.input "")}}
        {:type :terminal/read}]})

(fn correlated? [state event]
  (and state (= state.id event.id) (= state.correlation event.correlation)))

(fn open [db event]
  (assert (not db.dialog) "a dialog is already open")
  (assert (and (= (type event.id) :string) (not= event.id "")
               (= (type event.correlation) :string) (not= event.correlation ""))
          "invalid dialog identity")
  (assert (and (= (type event.completion) :string) (not= event.completion ""))
          "invalid dialog completion")
  (let [kind (or event.kind :modal)]
    (assert (or (= kind :modal) (= kind :progress) (= kind :alert))
            "invalid dialog kind")
    (let [state {:scroll 0
                 :actions (copy-actions event.actions)
                 :cancellable (= event.cancellable true)
                 :code event.code
                 :completion event.completion
                 :correlation event.correlation
                 :hints (or event.hints {})
                 :id event.id
                 :input (if event.protected "" (or event.initial ""))
                 :input_enabled (= event.input true)
                 :input_length 0
                 :kind kind
                 :message (or event.message "")
                 :progress event.progress
                 :sections event.sections
                 :protected (= event.protected true)
                 :title (or event.title "")
                 :url event.url}]
      (updated state {} (if event.protected
                            [{:type :terminal/read}
                             {:completion :dialog/protected-input
                              :correlation event.correlation
                              :id event.id
                              :type :input/protected}]
                            [{:type :terminal/read}])))))

(fn update [db event]
  (let [state db.dialog]
    (when (correlated? state event)
      (let [patch {}]
        (each [_ name (ipairs [:kind
                               :title
                               :message
                               :url
                               :code
                               :progress
                               :cancellable
                               :hints
                               :sections])]
          (when (not= (. event name) nil)
            (tset patch name (misa.replace (. event name)))))
        (when (not= event.actions nil)
          (tset patch :actions (misa.replace (copy-actions event.actions))))
        (when (not= event.input nil)
          (tset patch :input_enabled (= event.input true)))
        (updated state patch)))))

(fn close [db event]
  (when (and db.dialog (= db.dialog.id event.id)
             (or (not event.correlation)
                 (= db.dialog.correlation event.correlation)))
    {:patch {:dialog misa.delete} :fx [{:type :terminal/read}]}))

(fn protected-input [db event]
  (let [state db.dialog]
    (when (and (correlated? state event) state.protected)
      (if (or event.submitted event.cancelled)
          (completed state (if event.cancelled :cancel :submit) event.cancelled
                     false)
          (updated state
                   {:input_length (or event.length 0)
                    :input_error (misa.replace (when event.too_long
                                                 "Input is too long; shorten it before submitting."))})))))

(fn activate [state id]
  (if (and (= id :dialog-close) state.cancellable (not state.confirmation))
      (completed state :cancel true false)
      state.confirmation
      (if (= id :cancel-confirmation)
          (updated state {:confirmation misa.delete})
          (= id :confirm)
          (let [wanted state.confirmation.action]
            ;; Recheck current availability after asynchronous plan updates.
            (var current nil)
            (each [_ action (ipairs state.actions)]
              (when (and (= action.id wanted.id) (not action.disabled))
                (set current action)))
            (if current (completed state current.id false current.persistent)
                (updated state {:confirmation misa.delete})))
          {:fx [{:type :terminal/read}]})
      (do
        (var selected nil)
        (each [_ action (ipairs state.actions)]
          (when (and (= action.id id) (not action.disabled))
            (set selected action)))
        (if (not selected) {:fx [{:type :terminal/read}]} selected.confirm
            (updated state
                     {:confirmation (misa.replace {:action selected
                                                   :title (or selected.confirm.title
                                                              "Confirm action")
                                                   :message (or selected.confirm.message
                                                                "")
                                                   :label (or selected.confirm.label
                                                              selected.label)})})
            (completed state selected.id false selected.persistent)))))

(fn cancel [state]
  (if state.confirmation (updated state {:confirmation misa.delete})
      state.cancellable (completed state :cancel true false)
      {:fx [{:type :terminal/read}]}))

(fn enter [state]
  (var primary nil)
  (each [_ action (ipairs state.actions)]
    (when (and action.primary (not action.disabled))
      (set primary action.id)))
  (if primary (activate state primary)
      state.input_enabled (completed state :submit false false)
      {:fx [{:type :terminal/read}]}))

(fn backspace [state]
  (if state.input_enabled
      (let [text state.input]
        (var at (length text))
        (while (and (> at 0) (>= (text:byte at) 128) (< (text:byte at) 192))
          (set at (- at 1)))
        (updated state {:input (text:sub 1 (math.max 0 (- at 1)))}))
      {:fx [{:type :terminal/read}]}))

(fn text-2 [state event]
  (if state.input_enabled
      (updated state {:input (.. state.input (or event.text ""))})
      {:fx [{:type :terminal/read}]}))

(local inputs {:escape cancel
               :ctrl_c cancel
               :ctrl_d cancel
               :eof cancel
               :text text-2
               :backspace backspace
               :enter enter})

(local scroll-keys {:arrow_up -1
                    :arrow_down 1
                    :wheel_up -3
                    :wheel_down 3
                    :page_up :page-up
                    :page_down :page-down
                    :home :start
                    :end :end})

(fn scroll [db state direction cofx]
  ;; Share measured viewport geometry with whichever dialog component is selected.
  (let [terminal (or (and cofx cofx.terminal) {:columns 80 :lines 24})
        room (if (and misa.ui misa.ui.overlay-room)
                 (misa.ui.overlay-room db terminal)
                 terminal.lines)
        rendered (misa.dialogs.layout db
                                      {:terminal terminal
                                       :available_lines room})
        maximum (math.max 0 (or rendered.max_scroll 0))
        current (math.max 0 (math.min maximum (or state.scroll 0)))
        page (math.max 1 (or rendered.scroll_page (- room 3)))
        target (if (= direction :start) 0
                   (= direction :end) maximum
                   (= direction :page-up) (- current page)
                   (= direction :page-down) (+ current page)
                   (+ current direction))]
    (updated state {:scroll (math.max 0 (math.min maximum target))})))

(fn input [db event cofx]
  (let [state (assert db.dialog)]
    (var bound nil)
    (each [_ action (ipairs state.actions)]
      (when (and action.binding misa.keybindings misa.keybindings.action
                 (= (misa.keybindings.action action.binding.context event)
                    action.binding.action))
        (assert (not bound) "ambiguous dialog action binding")
        (set bound action.id)))
    (if state.protected {:fx [{:type :terminal/read}]} state.confirmation
        (if (= event.kind :enter) (activate state :confirm)
            (or (= event.kind :escape) (= event.kind :ctrl_c)
                (= event.kind :ctrl_d) (= event.kind :eof)) (cancel state)
            {:fx [{:type :terminal/read}]}) bound (activate state bound)
        (and (not state.input_enabled)
             (. scroll-keys (or event.key event.kind)))
        (scroll db state (. scroll-keys (or event.key event.kind)) cofx)
        (let [handler (. (misa.catalog :dialog-inputs) event.kind)]
          (if handler (handler state event) {:fx [{:type :terminal/read}]})))))

(fn on-dialog-action [db event]
  (if (and (correlated? db.dialog event) (not db.dialog.protected))
      (activate db.dialog event.action)
      {:fx [{:type :terminal/read}]}))

(fn route-ui-action [state event]
  ;; While modal, suppress underlying view actions as well.
  (var selected nil)
  (when (not state.protected)
    (each [_ action (ipairs (buttons state))]
      (when (= event.action (token state action))
        (set selected action.id))))
  {:type :dialog/action
   :id state.id
   :correlation state.correlation
   :action selected})

(fn dialog-inputs [_ handler]
  (assert (= (type handler) :function) "transition must be a function"))

(fn build []
  "Build the declarations for dialogs."
  (let [fx [{:catalog :services :id :dialogs.enabled? :value true}
            {:catalog :services :id :dialogs.action-token :value token}
            {:catalog :services :id :dialogs.buttons :value buttons}
            (let [definition {:id :dialogs/input
                              :event :terminal/input
                              :priority 1000
                              :context [:db/path :dialog]
                              :resolve (fn [_ event]
                                         (misa.patch event
                                                     {:type :dialog/input}))}]
              {:catalog :routes :id (. definition :id) :value definition})
            (let [definition {:id :dialogs/action
                              :event :ui/action
                              :priority 1000
                              :context [:db/path :dialog]
                              :resolve route-ui-action}]
              {:catalog :routes :id (. definition :id) :value definition})
            {:catalog :events
             :value {:event :dialog/action :handler on-dialog-action}}]]
    (each [name handler (pairs {:dialog/open open
                                :dialog/update update
                                :dialog/close close
                                :dialog/protected-input protected-input
                                :dialog/input input})]
      (table.insert fx {:catalog :events :value {:event name :handler handler}}))
    (definitions.build :dialogs
      fx
      {:dialog-inputs inputs :validators {:dialog-inputs dialog-inputs}})))

{: build}
