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
               :value (or (and item item.value) misa.json_null)}
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
                (misa.choice_refresh session db))
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

{:setup (fn []
          (assert (and (and misa.choice_session misa.choice_action)
                       misa.choice_picker_layout)
                  "picker requires choices and choice_layout")
          (set misa.picker true)
          (misa.reg_event :picker/open
                          (fn [db event]
                            (assert (or (not db.picker) (= event.nested true))
                                    "a picker is already open")
                            (local state
                                   (open-state event db
                                               (or (and event.nested db.picker)
                                                   nil)))
                            (set db.picker
                                 (or (and event.choose_view
                                          (view-picker state db))
                                     state))
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :picker/update
                          (fn [db event]
                            (var state db.picker)
                            (if (or (or (not state) (not= event.id state.id))
                                    (not= event.token state.token))
                                nil (do
                                     (if event.items
                                         (misa.choice_set_items state.session
                                                                event.items db)
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
                                                              db state.parent))
                                           (set db.picker replacement)
                                           (set state replacement)))
                                     (when (not= event.selected nil)
                                       (set state.session.selected
                                            (or (and (not= event.selected
                                                           misa.json_null)
                                                     event.selected)
                                                nil))
                                       (misa.choice_refresh state.session db))
                                     {: db}))))
          (misa.reg_interceptor {:before (fn [tx]
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
                                 :id :picker/input})
          (misa.reg_event :picker/input
                          (fn [db event cofx]
                            (local state (assert db.picker))
                            (local action (misa.choice_action event))
                            (local geometry
                                   (misa.choice_picker_layout state.session db
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
                            (if result.replace_view
                                (set db.picker (view-picker state db))
                                (and (and result.accepted
                                          (= state.id :picker-picker))
                                     state.parent)
                                (do
                                  (local parent state.parent)
                                  (misa.choice_replace_view parent.session
                                                            result.accepted.value
                                                            db)
                                  (set db.picker parent))
                                (and (and result.cancelled
                                          (= state.id :picker-picker))
                                     state.parent)
                                (set db.picker state.parent) result.accepted
                                (do
                                  (set db.picker state.parent)
                                  (let [___antifnl_rtn_1___ {: db
                                                             :fx (finish state
                                                                         result.accepted
                                                                         false)}]
                                    (lua "return ___antifnl_rtn_1___")))
                                result.cancelled
                                (do
                                  (set db.picker state.parent)
                                  (let [___antifnl_rtn_1___ {: db
                                                             :fx (finish state
                                                                         nil
                                                                         true)}]
                                    (lua "return ___antifnl_rtn_1___")))
                                result.favorite
                                (let [___antifnl_rtn_1___ {: db
                                                           :fx [{:event {:scope state.session.preference_scope
                                                                         :type :preferences/toggle
                                                                         :value result.favorite}
                                                                 :type :dispatch}
                                                                {:type :terminal/read}]}]
                                  (lua "return ___antifnl_rtn_1___")))
                            {: db :fx [{:type :terminal/read}]}))
          nil)}

