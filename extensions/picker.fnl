(local definitions (require :misa.definitions))

;; Overlay adapter for shared choice sessions. Geometry is supplied to the
;; shared positional resolver; picker-specific code owns only modal lifecycle.

(fn view-definitions [event]
  (if (= event.panels nil) nil (do
                                 (assert (and (= (type event.panels) :table)
                                              (> (length event.panels) 0))
                                         "picker panels must be a nonempty array")
                                 (local result {})
                                 (each [index panel (ipairs event.panels)]
                                   (tset result index
                                         {:id (or panel.id (tostring index))
                                          :items (or panel.items {})
                                          :title (or panel.title panel.id
                                                     (tostring index))}))
                                 result)))

(fn finish [state item cancelled]
  (let [event {:cancelled (= cancelled true)
               :choice_id (or (and item item.id) nil)
               :invocation (and item item.invocation)
               :picker state.id
               :picker_token state.token
               :type state.completion
               :value (if (and item (not= item.value nil)) item.value
                          misa.json-null)}
        fx [{: event :type :dispatch}]]
    (when (and item (not item.invocation) state.session.preference_scope)
      (tset fx (+ (length fx) 1)
            {:event {:scope state.session.preference_scope
                     :type :choice/used
                     :value item.id}
             :type :dispatch}))
    fx))

(fn open-state [event db parent]
  (assert (and (= (type event.id) :string) (not= event.id "")
               (= (type event.token) :string) (not= event.token ""))
          "invalid picker identity")
  (assert (and (= (type event.completion) :string) (not= event.completion ""))
          "invalid picker completion")
  (var session event.session)
  (if session (do
                (assert (and (= (type session) :table)
                             (= (type session.title) :string)
                             (= (type session.panels) :table))
                        "invalid choice session")
                (set session (misa.choices.refresh session db)))
      (let [defs (view-definitions event)]
        (var items (or event.items {}))
        (when defs
          (set items {})
          (local seen {})
          (each [_ definition (ipairs defs)]
            (each [_ item (ipairs definition.items)]
              (when (not (. seen item.value))
                (tset seen item.value true)
                (tset items (+ (length items) 1) item)))))
        (set session (misa.choices.session {: items
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
               :items (misa.choices.registered-views parent.session)
               :title "Choose view"
               :token (.. parent.token ":views")
               :views [:all]} db parent))

(fn updated [picker fx]
  {:patch {:picker (misa.replace picker)} :fx (or fx [{:type :terminal/read}])})

;; Sessions carry projected catalogues as well as tiny navigation fields.
;; Replacing the whole picker revalidates every unchanged catalogue branch.
;; Emit the fields the session owner actually changed; normal patch validation
;; still checks every incoming field before the native transaction commits.
(fn fields-changed [previous current]
  (local patch {})
  (each [key value (pairs current)]
    (when (not= value (. previous key)) (tset patch key (misa.replace value))))
  (each [key _ (pairs previous)]
    (when (= (. current key) nil) (tset patch key misa.delete)))
  patch)

(fn updated-session [state session fx]
  (local patch (fields-changed state.session session))
  (when (not= state.session.view_state session.view_state)
    (local states {})
    (each [id value (pairs session.view_state)]
      (when (not= value (. state.session.view_state id))
        (tset states id (fields-changed (or (. state.session.view_state id) {})
                                        value))))
    (each [id _ (pairs state.session.view_state)]
      (when (= (. session.view_state id) nil) (tset states id misa.delete)))
    (set patch.view_state states))
  {:patch {:picker {:session patch}} :fx (or fx [{:type :terminal/read}])})

(fn []
  "Build the declarations for picker."
  (local declarations [])
  (table.insert declarations {:catalog :services
                              :id :picker.enabled?
                              :value true})
  (table.insert declarations
                {:catalog :events
                 :value {:event :picker/open
                         :handler (fn [db event]
                                    (assert (or (not db.picker)
                                                (= event.nested true))
                                            "a picker is already open")
                                    (local state
                                           (open-state event db
                                                       (or (and event.nested
                                                                db.picker)
                                                           nil)))
                                    (updated (if event.choose_view
                                                 (view-picker state db)
                                                 state)))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :picker/update
                         :handler (fn [db event]
                                    (var state db.picker)
                                    (if (or (not state)
                                            (not= event.id state.id)
                                            (not= event.token state.token))
                                        nil
                                        (do
                                          (if event.items
                                              (set state
                                                   (misa.patch state
                                                               {:session (misa.replace (misa.choices.set-items state.session
                                                                                                               event.items
                                                                                                               db))}))
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
                                            (local selected
                                                   (when (not= event.selected
                                                               misa.json-null)
                                                     event.selected))
                                            (local session
                                                   (misa.patch state.session
                                                               {:selected (misa.replace selected)}))
                                            (set state
                                                 (misa.patch state
                                                             {:session (misa.replace (misa.choices.refresh session
                                                                                                           db))})))
                                          (updated state []))))}})
  (table.insert declarations
                (let [definition {:id :picker/input
                                  :event :terminal/input
                                  :priority 800
                                  :context [:db/path :picker]
                                  :resolve (fn [_ event]
                                             (misa.patch event
                                                         {:type :picker/input}))}]
                  {:catalog :routes :id (. definition :id) :value definition}))
  (table.insert declarations
                {:catalog :events
                 :value {:event :picker/input
                         :handler (fn [db event cofx]
                                    (local state (assert db.picker))
                                    (local action (misa.choices.action event))
                                    (local geometry
                                           (when (misa.choices.needs-targets? state.session
                                                                              event)
                                             (misa.choices.picker-layout state.session
                                                                         db
                                                                         cofx.terminal)))
                                    (local result
                                           (misa.choices.input state.session
                                                               {: action
                                                                :kind event.kind
                                                                :text event.text
                                                                :key event.key
                                                                :targets (and geometry
                                                                              geometry.targets)}
                                                               db))

                                    (fn next-state []
                                      (misa.patch state
                                                  {:session (misa.replace result.session)}))

                                    (if result.replace_view
                                        (updated (view-picker (next-state) db))
                                        (and (= state.id :picker-picker)
                                             state.parent result.accepted)
                                        (updated (misa.patch state.parent
                                                             {:session (misa.replace (misa.choices.replace-view state.parent.session
                                                                                                                result.accepted.value
                                                                                                                db))}))
                                        (and (= state.id :picker-picker)
                                             state.parent result.cancelled)
                                        (updated state.parent)
                                        result.accepted
                                        (updated state.parent
                                                 (finish (next-state)
                                                         result.accepted false))
                                        result.cancelled
                                        (updated state.parent
                                                 (finish (next-state) nil true))
                                        result.favorite
                                        (updated-session state result.session
                                                         [{:event {:scope result.session.preference_scope
                                                                   :type :preferences/toggle
                                                                   :value result.favorite}
                                                           :type :dispatch}
                                                          {:type :terminal/read}])
                                        (updated-session state result.session)))}})
  (definitions :picker
    declarations
    {:requirements {:picker.enabled? [:choices.action
                                      :choices.picker-layout
                                      :choices.session]}}))
