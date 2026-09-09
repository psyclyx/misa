;; Discoverable UI invocations. The palette uses ordinary replaceable choices;
;; feature owners register actions and retain all execution behavior.

(fn hover [_ event]
  "Record the hovered action and link."
  {:patch {:hover_action (if (and (= (type event.action) :string)
                                  (not= event.action ""))
                             event.action
                             misa.delete)
           :hover_link (if (and (= (type event.link) :string)
                                (not= event.link ""))
                           event.link
                           misa.delete)}})

(fn invoke-action [db event]
  "Dispatch an available action and resume terminal input."
  (let [action (misa.actions.lookup event.action)]
    (if (and (not (and misa.choices misa.choices.pending
                       (misa.choices.pending db))) action
             (or (not action.available) (action.available db)))
        {:fx [{:event (misa.snapshot action.event) :type :dispatch}
              {:type :terminal/read}]}
        {:fx [{:type :terminal/read}]})))

(fn palette-shortcut [_ event]
  "Resolve the global action-palette shortcut."
  (when (= (misa.keybindings.action :global event) :action_palette)
    {:type :actions/open}))

(fn open-palette [db event]
  "Open a choice session containing available actions."
  (if (or (and db.picker (= db.picker.id :actions)) db.dialog)
      {:fx [{:type :terminal/read}]}
      (let [items {}]
        (each [_ action (ipairs (misa.actions.all))]
          (when (and (or (not db.picker)
                         (and action.binding
                              (or (= action.binding.context :choices)
                                  (= action.binding.context :global))))
                     (or (not action.available) (action.available db)))
            (let [key (and action.binding
                           (misa.keybindings.hint action.binding.context
                                                  action.binding.action))]
              (tset items (+ (length items) 1)
                    {:description (.. (or (and key
                                               (.. (misa.keybindings.text key)
                                                   "  "))
                                          "")
                                      (or action.description ""))
                     :id action.id
                     :label action.label
                     :search action.id
                     :value action.id}))))
        (let [sequence (+ (or db.action_sequence 0) 1)]
          {:patch {:action_sequence sequence}
           :fx [{:event {:completion :actions/selected
                         :id :actions
                         :nested (not= db.picker nil)
                         :session (misa.choices.session {: items
                                                         :preference_scope :actions
                                                         :purpose :actions
                                                         :query (or event.query
                                                                    "")
                                                         :title ": Actions"}
                                                        db)
                         :title ": Actions"
                         :token (tostring sequence)
                         :type :picker/open}
                 :type :dispatch}]}))))

(fn select-action [db event]
  "Dispatch the selected action when its picker token matches."
  (if (or (not= event.picker :actions)
          (not= event.picker_token (tostring db.action_sequence)))
      nil
      (let [action (and (not event.cancelled) (misa.actions.lookup event.value))
            fx [{:type :terminal/read}]]
        (when (and action (or (not action.available) (action.available db)))
          (table.insert fx 1 {:event (misa.snapshot action.event)
                              :type :dispatch}))
        {: fx})))

(fn resolve-global-input [db event]
  "Resolve global actions and the colon palette shortcut."
  (when (and (not db.dialog) (not db.picker))
    (let [bound (misa.keybindings.action :global event)]
      (var selected nil)
      (when bound
        (each [_ action (ipairs (misa.actions.all))]
          (when (and action.binding (= action.binding.context :global)
                     (= action.binding.action bound)
                     (or (not action.available) (action.available db)))
            (assert (= selected nil) "ambiguous global action binding")
            (set selected action.event))))
      (let [editor (or db.editor {})]
        (or selected
            (when (and (= event.kind :text) (= (event.text:sub 1 1) ":")
                       (or (= (or editor.text "") "") (= editor.mode :normal)))
              {:type :actions/open :query (event.text:sub 2)}))))))

{:hover hover
 :resolve-global-input resolve-global-input
 :select-action select-action
 :palette-shortcut palette-shortcut
 :invoke-action invoke-action
 :open-palette open-palette}
