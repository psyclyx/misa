(local definitions (require :misa.definitions))

;; Discoverable UI invocations. The palette uses ordinary replaceable choices;
;; feature owners register actions and retain all execution behavior.

(fn []
  "Build the declarations for actions."
  (definitions :actions
    [{:catalog :events
      :value {:event :ui/hover
              :handler (fn [_ event]
                         {:patch {:hover_action (if (and (= (type event.action)
                                                            :string)
                                                         (not= event.action ""))
                                                    event.action
                                                    misa.delete)
                                  :hover_link (if (and (= (type event.link)
                                                          :string)
                                                       (not= event.link ""))
                                                  event.link
                                                  misa.delete)}})}}
     {:catalog :events
      :value {:event :ui/action
              :handler (fn [db event]
                         (local action (misa.actions.lookup event.action))
                         (if (and (not (and misa.choices misa.choices.pending
                                            (misa.choices.pending db)))
                                  action
                                  (or (not action.available)
                                      (action.available db)))
                             {:fx [{:event (misa.snapshot action.event)
                                    :type :dispatch}
                                   {:type :terminal/read}]}
                             {:fx [{:type :terminal/read}]}))}}
     (let [definition {:action :action_palette :context :global :default [:f1]}]
       {:catalog :keybindings
        :id (.. (. definition :context) "/" (. definition :action))
        :value definition})
     (let [definition {:binding {:action :action_palette :context :global}
                       :event {:type :actions/open}
                       :id :actions.open
                       :label "Open action palette / key reference"}]
       {:catalog :actions :id (. definition :id) :value definition})
     (let [definition {:id :actions/picker-palette
                       :event :terminal/input
                       :priority 900
                       :context [:db/path :picker]
                       :resolve (fn [_ event]
                                  (when (= (misa.keybindings.action :global
                                                                    event)
                                           :action_palette)
                                    {:type :actions/open}))}]
       {:catalog :routes :id (. definition :id) :value definition})
     (let [definition {:id :actions/input
                       :event :terminal/input
                       :priority 700
                       :context [:db/path]
                       :resolve (fn [db event]
                                  (when (and (not db.dialog) (not db.picker))
                                    (local bound
                                           (misa.keybindings.action :global
                                                                    event))
                                    (var selected nil)
                                    (when bound
                                      (each [_ action (ipairs (misa.actions.all))]
                                        (when (and action.binding
                                                   (= action.binding.context
                                                      :global)
                                                   (= action.binding.action
                                                      bound)
                                                   (or (not action.available)
                                                       (action.available db)))
                                          (assert (= selected nil)
                                                  "ambiguous global action binding")
                                          (set selected action.event))))
                                    (local editor (or db.editor {}))
                                    (or selected
                                        (when (and (= event.kind :text)
                                                   (= (event.text:sub 1 1) ":")
                                                   (or (= (or editor.text "")
                                                          "")
                                                       (= editor.mode :normal)))
                                          {:type :actions/open
                                           :query (event.text:sub 2)}))))}]
       {:catalog :routes :id (. definition :id) :value definition})
     {:catalog :events
      :value {:event :actions/open
              :handler (fn [db event]
                         (if (or (and db.picker (= db.picker.id :actions))
                                 db.dialog)
                             {:fx [{:type :terminal/read}]}
                             (do
                               (local items {})
                               (each [_ action (ipairs (misa.actions.all))]
                                 (when (and (or (not db.picker)
                                                (and action.binding
                                                     (or (= action.binding.context
                                                            :choices)
                                                         (= action.binding.context
                                                            :global))))
                                            (or (not action.available)
                                                (action.available db)))
                                   (local key
                                          (and action.binding
                                               (misa.keybindings.hint action.binding.context
                                                                      action.binding.action)))
                                   (tset items (+ (length items) 1)
                                         {:description (.. (or (and key
                                                                    (.. (misa.keybindings.text key)
                                                                        "  "))
                                                               "")
                                                           (or action.description
                                                               ""))
                                          :id action.id
                                          :label action.label
                                          :search action.id
                                          :value action.id})))
                               (local sequence (+ (or db.action_sequence 0) 1))
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
                                      :type :dispatch}]})))}}
     {:catalog :events
      :value {:event :actions/selected
              :handler (fn [db event]
                         (if (or (not= event.picker :actions)
                                 (not= event.picker_token
                                       (tostring db.action_sequence)))
                             nil
                             (do
                               (local action
                                      (and (not event.cancelled)
                                           (misa.actions.lookup event.value)))
                               (local fx [{:type :terminal/read}])
                               (when (and action
                                          (or (not action.available)
                                              (action.available db)))
                                 (table.insert fx 1
                                               {:event (misa.snapshot action.event)
                                                :type :dispatch}))
                               {: fx})))}}]
    {}))
