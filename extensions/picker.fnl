;; Overlay adapter for shared choice sessions. Geometry is supplied to the

;; shared positional resolver; picker-specific code owns only modal lifecycle.

(fn definitions [event]
  (if (= event.panels nil) nil (do
                                 (assert (and (= (type event.panels) :table)
                                              (> (length event.panels) 0))
                                         "picker panels must be a nonempty array")
                                 (local result {})
                                 (each [index panel (ipairs event.panels)]
                                   (tset result index
                                         {:id (or panel.id (tostring index))
                                          :items (or panel.items {})
                                          :title (or (or panel.title panel.id)
                                                     (tostring index))}))
                                 result)))

(fn finish [state item cancelled]
  (let [event {:cancelled (= cancelled true)
               :choice_id (or (and item item.id) nil)
               :invocation (and item item.invocation)
               :picker state.id
               :picker_token state.token
               :type state.completion
               :value (if (and item (not= item.value nil)) item.value misa.json_null)}
        fx [{: event :type :dispatch}]]
    (when (and (and item (not item.invocation)) state.session.preference_scope)
      (tset fx (+ (length fx) 1)
            {:event {:scope state.session.preference_scope
                     :type :choice/used
                     :value item.id}
             :type :dispatch}))
    fx))

(fn open-state [event db parent]
  (assert (and (and (and (= (type event.id) :string) (not= event.id ""))
                    (= (type event.token) :string))
               (not= event.token "")) "invalid picker identity")
  (assert (and (= (type event.completion) :string) (not= event.completion ""))
          "invalid picker completion")
  (var session event.session)
  (if session (do
                (assert (and (and (= (type session) :table)
                                  (= (type session.title) :string))
                             (= (type session.panels) :table))
                        "invalid choice session")
                (set session (misa.choice_refresh session db)))
      (let [defs (definitions event)]
        (var items (or event.items {}))
        (when defs
          (set items {})
          (local seen {})
          (each [_ definition (ipairs defs)]
            (each [_ item (ipairs definition.items)]
              (when (not (. seen item.value))
                (tset seen item.value true)
                (tset items (+ (length items) 1) item)))))
        (set session (misa.choice_session {: items
                                           :preference_scope event.preference_scope
                                           :purpose (or event.purpose :generic)
                                           :selected event.selected
                                           :title event.title
                                           :view_definitions defs
                                           :views event.views}
                                          db))))
  {:completion event.completion
   :id event.id
   : parent
   : session
   :token event.token})

(fn view-picker [parent db]
  (open-state {:completion :picker/replace-view
               :id :picker-picker
               :items (misa.choice_registered_views parent.session)
               :title "Choose view"
               :token (.. parent.token ":views")
               :views [:all]} db parent))

(fn updated [picker fx]
  {:patch {:picker (misa.replace picker)} :fx (or fx [{:type :terminal/read}])})

{:setup (fn []
          (local setup-fx [])
          (assert (and (and misa.choice_session misa.choice_action)
                       misa.choice_picker_layout)
                  "picker requires choices and choice_layout")
          (table.insert setup-fx {:type :register/service
                                  :name :picker
                                  :value true})
          (table.insert setup-fx
                        {:type :register/event
                         :name :picker/open
                         :handler (fn [db event]
                                    (assert (or (not db.picker)
                                                (= event.nested true))
                                            "a picker is already open")
                                    (local state
                                           (open-state event db
                                                       (or (and event.nested
                                                                db.picker)
                                                           nil)))
                                    (updated (if event.choose_view (view-picker state db) state)))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :picker/update
                         :handler (fn [db event]
                                    (var state db.picker)
                                    (if (or (or (not state)
                                                (not= event.id state.id))
                                            (not= event.token state.token))
                                        nil
                                        (do
                                          (if event.items
                                              (set state
                                                   (misa.patch state
                                                               {:session (misa.replace (misa.choice_set_items state.session
                                                                                                             event.items db))}))
                                              event.panels
                                              (do
                                                (local replacement
                                                       (open-state {:completion state.completion
                                                                    :id state.id
                                                                    :panels event.panels
                                                                    :preference_scope state.session.preference_scope
                                                                    :purpose state.session.purpose
                                                                    :selected (or (and (not= event.selected
                                                                                             nil)
                                                                                       event.selected)
                                                                                  state.session.selected)
                                                                    :title state.session.title
                                                                    :token state.token}
                                                                   db
                                                                   state.parent))
                                                (set state replacement)))
                                          (when (not= event.selected nil)
                                            (local selected (when (not= event.selected misa.json_null)
                                                              event.selected))
                                            (local session (misa.patch state.session {:selected (misa.replace selected)}))
                                            (set state
                                                 (misa.patch state {:session (misa.replace (misa.choice_refresh session db))})))
                                          (updated state []))))})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (when (and (= tx.event.type
                                                         :terminal/input)
                                                      tx.db.picker)
                                             (set tx.event
                                                  {:action tx.event.action
                                                   :key tx.event.key
                                                   :kind tx.event.kind
                                                   :text tx.event.text
                                                   :type :picker/input}))
                                           tx)
                                 :id :picker/input}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :picker/input
                         :handler (fn [db event cofx]
                                    (local state (assert db.picker))
                                    (local action (misa.choice_action event))
                                    (local geometry
                                           (misa.choice_picker_layout state.session
                                                                      db
                                                                      cofx.terminal))
                                    (local item (. geometry.targets action))
                                    (local result
                                           (or (and item
                                                    (misa.choice_accept state.session
                                                                        item db))
                                               (misa.choice_input state.session
                                                                  {: action
                                                                   :kind event.kind
                                                                   :text event.text}
                                                                  db)))
                                    (local next-state (misa.patch state {:session (misa.replace result.session)}))
                                    (if result.replace_view
                                        (updated (view-picker next-state db))
                                        (and (= state.id :picker-picker) state.parent result.accepted)
                                        (updated (misa.patch state.parent
                                                             {:session (misa.replace (misa.choice_replace_view state.parent.session
                                                                                                              result.accepted.value db))}))
                                        (and (= state.id :picker-picker) state.parent result.cancelled)
                                        (updated state.parent)
                                        result.accepted
                                        (updated state.parent (finish next-state result.accepted false))
                                        result.cancelled
                                        (updated state.parent (finish next-state nil true))
                                        result.favorite
                                        (updated next-state
                                                 [{:event {:scope next-state.session.preference_scope
                                                           :type :preferences/toggle
                                                           :value result.favorite}
                                                   :type :dispatch}
                                                  {:type :terminal/read}])
                                        (updated next-state)))})
          {:fx setup-fx})}
