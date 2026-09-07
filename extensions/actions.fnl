;; Discoverable UI invocations. The palette uses ordinary replaceable choices;

;; feature owners register actions and retain all execution behavior.

{:setup (fn []
          {:fx [{:type :register/event
                 :name :ui/hover
                 :handler (fn [_ event]
                            {:patch {:hover_action
                                     (if (and (= (type event.action) :string)
                                              (not= event.action ""))
                                         event.action misa.delete)
                                     :hover_link (if (and (= (type event.link) :string) (not= event.link ""))
                                                     event.link misa.delete)}})}
                {:type :register/event
                 :name :ui/action
                 :handler (fn [db event]
                            (local action (misa.action event.action))
                            (if (and action
                                     (or (not action.available)
                                         (action.available db)))
                                {:fx [{:event (misa.snapshot action.event)
                                       :type :dispatch}
                                      {:type :terminal/read}]}
                                {:fx [{:type :terminal/read}]}))}
                {:type :register/keybinding
                 :value {:action :action_palette
                         :context :global
                         :default [:f1]}}
                {:type :register/action
                 :value {:binding {:action :action_palette :context :global}
                         :event {:type :actions/open}
                         :id :actions.open
                         :label "Open action palette / key reference"}}
                {:type :register/event-route
                 :value {:id :actions/picker-palette :event :terminal/input :priority 900
                         :context [:db/path :picker]
                         :resolve (fn [_ event]
                                    (when (= (misa.keybinding_action :global event) :action_palette)
                                      {:type :actions/open}))}}
                {:type :register/event-route
                 :value {:id :actions/input :event :terminal/input :priority 700
                         :context [:db/path]
                         :resolve (fn [db event]
                                    (when (and (not db.dialog) (not db.picker))
                                      (local bound (misa.keybinding_action :global event))
                                      (var selected nil)
                                      (when bound
                                        (each [_ action (ipairs (misa.actions))]
                                          (when (and action.binding (= action.binding.context :global)
                                                     (= action.binding.action bound)
                                                     (or (not action.available) (action.available db)))
                                            (assert (= selected nil) "ambiguous global action binding")
                                            (set selected action.event))))
                                      (local editor (or db.editor {}))
                                      (or selected
                                          (when (and (= event.kind :text)
                                                     (= (event.text:sub 1 1) ":")
                                                     (or (= (or editor.text "") "") (= editor.mode :normal)))
                                            {:type :actions/open :query (event.text:sub 2)}))))}}
                {:type :register/event
                 :name :actions/open
                 :handler (fn [db event]
                            (if (or (and db.picker (= db.picker.id :actions))
                                    db.dialog)
                                {:fx [{:type :terminal/read}]}
                                (do
                                  (local items {})
                                  (each [_ action (ipairs (misa.actions))]
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
                                                  (misa.keybinding_hint action.binding.context
                                                                        action.binding.action)))
                                      (tset items (+ (length items) 1)
                                            {:description (.. (or (and key
                                                                       (.. (misa.keybinding_text key)
                                                                           "  "))
                                                                  "")
                                                              (or action.description
                                                                  ""))
                                             :id action.id
                                             :label action.label
                                             :search action.id
                                             :value action.id})))
                                  (local sequence
                                       (+ (or db.action_sequence 0) 1))
                                  {:patch {:action_sequence sequence}
                                   :fx [{:event {:completion :actions/selected
                                                 :id :actions
                                                 :nested (not= db.picker nil)
                                                 :session (misa.choice_session {: items
                                                                                :preference_scope :actions
                                                                                :purpose :actions
                                                                                :query (or event.query
                                                                                           "")
                                                                                :title ": Actions"}
                                                                               db)
                                                 :title ": Actions"
                                                 :token (tostring sequence)
                                                 :type :picker/open}
                                         :type :dispatch}]})))}
                {:type :register/event
                 :name :actions/selected
                 :handler (fn [db event]
                            (if (or (not= event.picker :actions)
                                    (not= event.picker_token
                                          (tostring db.action_sequence)))
                                nil
                                (do
                                  (local action
                                         (and (not event.cancelled)
                                              (misa.action event.value)))
                                  (local fx [{:type :terminal/read}])
                                  (when (and action
                                             (or (not action.available)
                                                 (action.available db)))
                                    (table.insert fx 1
                                                  {:event (misa.snapshot action.event)
                                                   :type :dispatch}))
                                  {: fx})))}]})}
