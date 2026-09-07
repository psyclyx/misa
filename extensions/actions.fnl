;; Discoverable UI invocations. The palette uses ordinary replaceable choices;

;; feature owners register actions and retain all execution behavior.

{:setup (fn []
          {:fx [{:type :register/event
                 :name :ui/hover
                 :handler (fn [_ event]
                            {:patch {:hover_action
                                     (if (and (= (type event.action) :string)
                                              (not= event.action ""))
                                         event.action misa.delete)}})}
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
                {:type :register/interceptor
                 :value {:before (fn [tx]
                                   (if (or (not= tx.event.type :terminal/input)
                                           tx.db.dialog)
                                       tx
                                       (if tx.db.picker
                                           (do
                                             (when (= (misa.keybinding_action :global
                                                                              tx.event)
                                                      :action_palette)
                                               (set tx.event
                                                    {:type :actions/open}))
                                             tx)
                                           (do
                                             (local editor (or tx.db.editor {}))
                                             (local bound
                                                    (misa.keybinding_action :global
                                                                            tx.event))
                                             (when bound
                                               (each [_ action (ipairs (misa.actions))]
                                                 (when (and (and (= action.binding.context
                                                                    :global)
                                                                 (= action.binding.action
                                                                    bound))
                                                            (or (not action.available)
                                                                (action.available tx.db)))
                                                   (set tx.event
                                                        (misa.snapshot action.event))
                                                   (lua "return tx"))))
                                             (when (and (and (= tx.event.kind
                                                                :text)
                                                             (= (tx.event.text:sub 1
                                                                                   1)
                                                                ":"))
                                                        (or (= (or editor.text
                                                                   "")
                                                               "")
                                                            (= editor.mode
                                                               :normal)))
                                               (set tx.event
                                                    {:query (or (and (= tx.event.kind
                                                                        :text)
                                                                     (tx.event.text:sub 2))
                                                                "")
                                                     :type :actions/open}))
                                             tx))))
                         :id :actions/input}}
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
