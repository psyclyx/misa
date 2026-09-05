;; Generic choice state and transitions shared by inline and overlay presentations.

(local (views sources) (values {} {}))

(local key-context :choices)

(local banks [[:1 :2 :3 :4 :5 :6 :7 :8 :9]
              [:q :w :e :r :t :y :u :i :o]
              [:a :s :d :f :g :h :j :k :l]])

(local fallback {:alt+/ :replace_view
                 :alt+p :cycle_previous
                 :alt+space :open_overlay
                 :alt+v :favorite
                 :arrow_down :next
                 :arrow_left :cycle_previous
                 :arrow_right :cycle
                 :arrow_up :previous
                 :ctrl_c :cancel
                 :ctrl_d :cancel
                 :enter :accept
                 :eof :cancel
                 :escape :cancel
                 :tab :accept})

(fn scalar [v]
  (or (or (= (type v) :string) (= (type v) :number)) (= (type v) :boolean)))

(fn clone [value]
  (if (not= (type value) :table) value
      (let [result {}]
        (each [key item (pairs value)] (tset result key (clone item)))
        result)))

(fn item [source]
  (assert (= (type source) :table) "choice item must be a table")
  (local value (or (and (not= source.value nil) source.value) source.id))
  (assert (scalar value) "choice value must be scalar")
  (local id (or source.id (or (and (= (type value) :string) value)
                              (tostring value))))
  (assert (and (= (type id) :string) (not= id "")) "choice id must be nonempty")
  (local display source.display)
  (var (label description) nil)
  (if (= (type display) :string) (set label display)
      (= (type display) :table) (set (label description)
                                     (values display.label display.description)))
  (set (label description)
       (values (or (or label source.label) id)
               (or description source.description)))
  (assert (and (= (type label) :string)
               (or (= description nil) (= (type description) :string)))
          "invalid choice display")
  (assert (or (or (= source.search nil) (= (type source.search) :string))
              (= (type source.search) :table))
          "choice search must be a string or array")
  (var path (or source.path id))
  (when (= (type path) :table) (set path (table.concat path "/")))
  (assert (= (type path) :string) "choice path must be a string or array")
  {: description
   : id
   :invocation source.invocation
   : label
   :narrow (clone source.narrow)
   : path
   :preview (clone source.preview)
   :search (clone source.search)
   :section source.section
   : value})

(fn items [source]
  (assert (= (type source) :table) "choice items must be an array")
  (local (result seen) (values {} {}))
  (each [_ raw (ipairs source)]
    (local value (item raw))
    (assert (not (. seen value.id)) "choice ids must be unique")
    (tset seen value.id true)
    (tset result (+ (length result) 1) value))
  result)

(fn search-text [value]
  (let [(fields seen) (values {} {})]
    (fn add [field]
      (when (and (and (= (type field) :string) (not= field ""))
                 (not (. seen field)))
        (tset seen field true)
        (tset fields (+ (length fields) 1) field)))

    (add value.id)
    (add value.path)
    (add value.label)
    (add value.description)
    (if (= (type value.search) :string) (add value.search)
        (= (type value.search) :table) (each [_ field (ipairs value.search)]
                                         (add field)))
    (table.concat fields " ")))

(fn matches [source query]
  (if (= query "") source (if misa.fuzzy_choices
                              (misa.fuzzy_choices source query search-text)
                              (let [(result needle) (values {} (query:lower))]
                                (each [_ value (ipairs source)]
                                  (when (or (= needle "")
                                            (: (: (search-text value) :lower)
                                               :find needle 1 true))
                                    (tset result (+ (length result) 1) value)))
                                result))))

(fn preference [db scope id]
  (let [scopes (and db.preferences db.preferences.scopes)]
    (and (and scopes (. scopes scope)) (. scopes scope id))))

(fn compatible [id session]
  (let [view (. views id)]
    (and view (or (not view.compatible) (view.compatible session)))))

(fn configured [purpose]
  (let [purposes (and (= (type misa.choice_config) :table)
                      misa.choice_config.purposes)]
    (var selected (or (and (= (type purposes) :table)
                           (or (. purposes purpose)
                               (. purposes (purpose:gsub "_" "-"))))
                      nil))
    (set selected (or selected [:all]))
    (assert (and (= (type selected) :table) (> (length selected) 0))
            "choice purpose must select at least one view")
    (local result {})
    (each [_ id (ipairs selected)]
      (assert (. views id) (.. "unknown choice view: " (tostring id)))
      (tset result (+ (length result) 1) id))
    result))

(fn project [session slot db]
  (let [id (. session.view_ids slot)
        definition (and session.custom_views (. session.custom_views slot))
        view (or definition (assert (. views id)))]
    (var source {})
    (if definition (set source (matches definition.items session.query))
        view.project (set source (view.project session matches db))
        (do
          (each [_ value (ipairs session.items)]
            (when (or (not view.include) (view.include value session db))
              (tset source (+ (length source) 1) value)))
          (when view.order
            (table.sort source (fn [a b] (view.order a b session db))))
          (set source (matches source session.query))))
    (local state (or (. session.view_state id) {:highlight 0}))
    (tset session.view_state id state)
    (local previous (and (and (= state.query session.query) state.items)
                         (. state.items state.highlight)))
    (set state.query session.query)
    (set (state.items state.highlight)
         (values source (or (and (> (length source) 0) 1) 0)))
    (when previous
      (var found false)
      (each [index value (ipairs source) &until found]
        (when (= value.id previous.id)
          (set state.highlight index)
          (set found true))))
    {:filtered source
     :highlight state.highlight
     : id
     :items source
     :title (or view.title id)}))

(fn refresh [session db]
  (set session.panels {})
  (for [slot 1 (length session.view_ids)]
    (tset session.panels slot (project session slot db)))
  session)

(fn pop-utf8 [value]
  (var at (length value))
  (while (and (and (> at 0) (>= (value:byte at) 128)) (< (value:byte at) 192))
    (set at (- at 1)))
  (value:sub 1 (math.max 0 (- at 1))))

(local positional-actions {})
(each [bank keys (ipairs banks)]
  (each [slot key (ipairs keys)]
    (tset positional-actions (.. :alt+ key) (.. :option_ bank "_" slot))))

(fn action [event]
  (if (= (type event.action) :string) event.action misa.keybinding_action
      (misa.keybinding_action key-context event)
      (let [key (or (and (= event.kind :key) event.key)
                    (and (= event.kind :alt) (.. :alt+ (event.text:lower)))
                    event.kind)]
        (or (. fallback key) (. positional-actions key)))))

(fn hint [name]
  (if misa.keybinding_hint (misa.keybinding_hint key-context name)
      (let [(bank-text slot-text) (name:match "^option_(%d+)_(%d+)$")
            bank (tonumber bank-text)
            slot (tonumber slot-text)]
        (if (and bank (. banks bank) (. banks bank slot))
            (.. :alt+ (. banks bank slot))
            (do
              (var result nil)
              (each [key value (pairs fallback) &until result]
                (when (= value name) (set result key)))
              result)))))

(fn first [panel room]
  (if (or (<= room 0) (<= (length panel.items) room))
      1
      (math.max 1
                (math.min (- panel.highlight (math.floor (/ room 2)))
                          (+ (- (length panel.items) room) 1)))))

(fn source-spec [id context db]
  (let [source (assert (. sources id)
                       (.. "unknown choice source: " (tostring id)))
        spec (or (source.items context db) {})]
    (if spec.items spec {:items spec})))

(fn apply-spec [session spec db]
  (set session.title (or spec.title session.title))
  (set session.purpose (or spec.purpose session.purpose))
  (set session.items (items (or spec.items {})))
  (set session.query (or spec.query ""))
  (set session.input_prefix (or spec.input_prefix ""))
  (if (= spec.selected misa.json_null) (set session.selected nil)
      (set session.selected spec.selected))
  (set session.preference_scope spec.preference_scope)
  (set session.view_state {})
  (set session.view_ids
       (or (and spec.views (clone spec.views)) (configured session.purpose)))
  (set session.custom_views nil)
  (refresh session db))

(local frame-keys [:title
                   :purpose
                   :items
                   :query
                   :input_prefix
                   :selected
                   :preference_scope
                   :view_ids
                   :custom_views
                   :view_state])

(fn push-narrow [session narrow db]
  (let [frame {}]
    (each [_ key (ipairs frame-keys)] (tset frame key (. session key)))
    (tset session.stack (+ (length session.stack) 1) frame)
    (local spec (or (and narrow.source
                         (source-spec narrow.source narrow.context db))
                    narrow))
    (apply-spec session spec db)
    {:consumed true :narrowed true}))

(fn pop-narrow [session db]
  (let [frame (table.remove session.stack)]
    (if (not frame)
        false
        (do
          (each [_ key (ipairs frame-keys)] (tset session key (. frame key)))
          (refresh session db)
          true))))

(fn new [spec db]
  (assert (and (and (= (type spec) :table) (= (type spec.title) :string))
               (not= spec.title ""))
          "choice session requires a title")
  (local session {:stack {}})
  (apply-spec session spec db)
  (when spec.view_definitions
    (set (session.view_ids session.custom_views) (values {} {}))
    (each [index definition (ipairs spec.view_definitions)]
      (tset session.view_ids index (assert definition.id))
      (tset session.custom_views index
            {:id definition.id
             :items (items (or definition.items {}))
             :title definition.title}))
    (refresh session db))
  session)

(fn row [value focused selected hotkey]
  (var description value.description)
  (when (or (or (= description value.label) (= description value.id))
            (= description (tostring value.value)))
    (set description nil))
  {:active focused
   : description
   : hotkey
   :id value.id
   :label value.label
   :marker (or (and focused ">") (or (and selected "✓") " "))
   :path value.path
   :preview value.preview
   :section value.section
   : selected
   :value value.value})

{:setup (fn [context]
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_config
                         :value (or (and (and (= (type context.config) :table)
                                              (= (type context.config.choices)
                                                 :table))
                                         context.config.choices)
                                    {})})
          (local declarations
                 {:accept [:enter :tab]
                  :cancel [:escape :ctrl_c :ctrl_d :eof]
                  :cycle [:arrow_right]
                  :cycle_previous [:arrow_left :alt+p]
                  :favorite [:alt+v]
                  :next [:arrow_down]
                  :open_overlay [:alt+space]
                  :previous [:arrow_up]
                  :replace_view [:alt+/]})
          (each [name keys (pairs declarations)]
            (table.insert setup-fx
                          {:type :register/keybinding
                           :value {:action name
                                   :context key-context
                                   :default keys}}))
          (each [bank keys (ipairs banks)]
            (each [slot key (ipairs keys)]
              (table.insert setup-fx
                            {:type :register/keybinding
                             :value {:action (.. :option_ bank "_" slot)
                                     :context key-context
                                     :default [(.. :alt+ key)]}})))
          (local labels
                 {:accept "Accept choice"
                  :cancel "Cancel choices / back"
                  :cycle "Next choice view"
                  :cycle_previous "Previous choice view"
                  :favorite "Toggle favorite choice"
                  :next "Next choice"
                  :open_overlay "Expand inline choices"
                  :previous "Previous choice"
                  :replace_view "Replace choice view"})
          (each [name label (pairs labels)]
            (local action-name name)
            (table.insert setup-fx
                          {:type :register/action
                           :value {:available (fn [db]
                                                (local session
                                                       (or (and db.picker
                                                                db.picker.session)
                                                           (and db.editor
                                                                db.editor.choice)))
                                                (and (and (not= session nil)
                                                          (or (not= action-name
                                                                    :favorite)
                                                              (not= session.preference_scope
                                                                    nil)))
                                                     (or (not= action-name
                                                               :open_overlay)
                                                         (= db.picker nil))))
                                   :binding {:action name :context key-context}
                                   :event {:action name
                                           :type :choices/dispatch}
                                   :id (.. :choices. name)
                                   : label}}))
          ;; Positional targets use the same geometry resolver for keyboard and pointer.
          ;; A stale or hidden button therefore cannot choose a different hidden row.
          (each [bank keys (ipairs banks)]
            (for [slot 1 (length keys)]
              (local name (.. :option_ bank "_" slot))
              (table.insert setup-fx
                            {:type :register/action
                             :value {:available (fn [db]
                                                  (or (or (not= db.picker nil)
                                                          (and db.editor
                                                               (not= db.editor.choice
                                                                     nil)))
                                                      false))
                                     :binding {:action name
                                               :context key-context}
                                     :event {:action name
                                             :type :choices/dispatch}
                                     :id (.. :choices. name)
                                     :label (.. "Choose visible item " bank ":"
                                                slot)}})))
          (table.insert setup-fx
                        {:type :register/event
                         :name :choices/dispatch
                         :handler (fn [db event]
                                    (if (and (not db.picker)
                                             (not (and db.editor
                                                       db.editor.choice)))
                                        {: db :fx [{:type :terminal/read}]}
                                        {: db
                                         :fx [{:event {:action event.action
                                                       :kind :choice_action
                                                       :type (or (and db.picker
                                                                      :picker/input)
                                                                 :terminal/input)}
                                               :type :dispatch}]}))})
          (table.insert setup-fx
                        {:type :register/setup-effect
                         :name :register/choice-view
                         :handler (fn [effect]
                                    (let [id effect.id
                                          view effect.value]
                                      (assert (and (= (type id) :string)
                                                   (not= id "")
                                                   (not (. views id)))
                                              "invalid or duplicate choice view")
                                      (tset views id (assert view))))})
          (table.insert setup-fx
                        {:type :register/setup-effect
                         :name :register/choice-source
                         :handler (fn [effect]
                                    (let [id effect.id
                                          source effect.value]
                                      (assert (and (= (type id) :string)
                                                   (not= id "")
                                                   (not (. sources id)))
                                              "invalid or duplicate choice source")
                                      (assert (and (= (type source) :table)
                                                   (= (type source.items)
                                                      :function))
                                              "choice source requires items")
                                      (tset sources id source)))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_source
                         :value (fn [id context-value db]
                                  (source-spec id context-value db))})
          (table.insert setup-fx
                        {:type :register/choice-view
                         :id :all
                         :value {:title :All}})
          (table.insert setup-fx
                        {:type :register/choice-view
                         :id :favorites
                         :value {:include (fn [value session db]
                                            (local p
                                                   (preference db
                                                               session.preference_scope
                                                               value.id))
                                            (and p (= p.favorite true)))
                                 :title :Favorites}})
          (table.insert setup-fx
                        {:type :register/choice-view
                         :id :frecency
                         :value {:include (fn [value session db]
                                            (local p
                                                   (preference db
                                                               session.preference_scope
                                                               value.id))
                                            (and p (> (or p.uses 0) 0)))
                                 :order (fn [a b session db]
                                          (local (pa pb)
                                                 (values (or (preference db
                                                                         session.preference_scope
                                                                         a.id)
                                                             {})
                                                         (or (preference db
                                                                         session.preference_scope
                                                                         b.id)
                                                             {})))
                                          (> (+ (* (or pa.last 0) 1000000)
                                                (or pa.uses 0))
                                             (+ (* (or pb.last 0) 1000000)
                                                (or pb.uses 0))))
                                 :title :Recent}})
          ;; Grouping is a view policy. The layout treats section names as ordinary
          ;; row metadata, so extensions can compose other grouped lists the same way.
          (table.insert setup-fx
                        {:type :register/choice-view
                         :id :browse
                         :value {:project (fn [session match-items db]
                                            (local all
                                                   (match-items session.items
                                                                session.query))
                                            (if (not= session.query "") all
                                                (do
                                                  (local recent {})
                                                  (each [_ value (ipairs all)]
                                                    (local p
                                                           (preference db
                                                                       session.preference_scope
                                                                       value.id))
                                                    (when (and p
                                                               (> (or p.uses 0)
                                                                  0))
                                                      (tset recent
                                                            (+ (length recent)
                                                               1)
                                                            value)))
                                                  (table.sort recent
                                                              (fn [a b]
                                                                (local (pa pb)
                                                                       (values (preference db
                                                                                           session.preference_scope
                                                                                           a.id)
                                                                               (preference db
                                                                                           session.preference_scope
                                                                                           b.id)))
                                                                (if (= pa.last
                                                                       pb.last)
                                                                    (< a.id
                                                                       b.id)
                                                                    (> (or pa.last
                                                                           0)
                                                                       (or pb.last
                                                                           0)))))
                                                  (local (result seen)
                                                         (values {} {}))
                                                  (local limit
                                                         (math.max 0
                                                                   (math.floor (or (tonumber misa.choice_config.recent_limit)
                                                                                   3))))
                                                  (for [index 1 (math.min (length recent)
                                                                          limit
                                                                          (math.max 0
                                                                                    (- (length all)
                                                                                       1)))]
                                                    (local value
                                                           (clone (. recent
                                                                     index)))
                                                    (set value.section :Recent)
                                                    (tset result
                                                          (+ (length result) 1)
                                                          value)
                                                    (tset seen value.id true))
                                                  (each [_ value (ipairs all)]
                                                    (when (not (. seen value.id))
                                                      (let [copy (clone value)]
                                                        (set copy.section :All)
                                                        (table.insert result
                                                                      copy))))
                                                  result)))
                                 :title :Browse}})
          (table.insert setup-fx {:type :register/service
                                  :name :choice_session
                                  :value new})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_refresh
                         :value refresh})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_action
                         :value action})
          (table.insert setup-fx {:type :register/service
                                  :name :choice_hint
                                  :value hint})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_first_index
                         :value first})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_set_items
                         :value (fn [session source-items db]
                                  (set session.items (items source-items))
                                  (refresh session db))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_replace_view
                         :value (fn [session id db]
                                  (assert (compatible id session)
                                          "incompatible choice view")
                                  (tset session.view_ids 1 id)
                                  (when session.custom_views
                                    (tset session.custom_views 1 false))
                                  (refresh session db))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_registered_views
                         :value (fn [session]
                                  (local result {})
                                  (each [id view (pairs views)]
                                    (when (compatible id session)
                                      (tset result (+ (length result) 1)
                                            {:display (or view.title id)
                                             : id
                                             :search id
                                             :value id})))
                                  (table.sort result (fn [a b] (< a.id b.id)))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_accept
                         :value (fn [session value db]
                                  (if value.narrow
                                      (push-narrow session value.narrow db)
                                      {:accepted value :consumed true}))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_rows
                         :value (fn [session db hotkeys]
                                  (refresh session db)
                                  (local result {})
                                  (each [bank panel (ipairs session.panels)]
                                    (local rows {})
                                    (each [index value (ipairs panel.items)]
                                      (tset rows index
                                            (row value
                                                 (and (= bank 1)
                                                      (= index panel.highlight))
                                                 (= value.value
                                                    session.selected)
                                                 (and (and hotkeys
                                                           (. hotkeys bank))
                                                      (. hotkeys bank index)))))
                                    (tset result bank
                                          {:id panel.id
                                           : rows
                                           :title panel.title}))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_hotkeys
                         :value (fn [session visible room]
                                  (local result {})
                                  (for [bank 1 (math.min (or visible 0)
                                                         (length session.panels)
                                                         (length banks))]
                                    (tset result bank {})
                                    (local panel (. session.panels bank))
                                    (local start (first panel room))
                                    (for [slot 1 (math.min room 9)]
                                      (when (. panel.items (- (+ start slot) 1))
                                        (tset (. result bank)
                                              (- (+ start slot) 1)
                                              (hint (.. :option_ bank "_" slot))))))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_positional
                         :value (fn [session name visible room]
                                  (var result nil)
                                  (for [bank 1 (math.min (or visible 0)
                                                         (length session.panels)
                                                         (length banks))
                                        &until result]
                                    (for [slot 1 (math.min (or room 0) 9)
                                          &until result]
                                      (when (= name (.. :option_ bank "_" slot))
                                        (set result
                                             (. session.panels bank :items
                                                (- (+ (first (. session.panels
                                                                bank)
                                                             room)
                                                      slot)
                                                   1))))))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :choice_input
                         :value (fn [session event db]
                                  (refresh session db)
                                  (let [panel (. session.panels 1)
                                        state (. session.view_state
                                                 (. session.view_ids 1))
                                        name event.action]
                                    (fn changed []
                                      (refresh session db)
                                      {:consumed true})

                                    (if (and (= event.kind :text)
                                             (= (type event.text) :string))
                                        (do
                                          (set session.query
                                               (.. session.query event.text))
                                          (changed))
                                        (= event.kind :backspace)
                                        (if (not= session.query "")
                                            (do
                                              (set session.query
                                                   (pop-utf8 session.query))
                                              (changed))
                                            (pop-narrow session db)
                                            {:consumed true :narrowed true}
                                            {:consumed false})
                                        (and (= name :previous)
                                             (> (length panel.items) 0))
                                        (do
                                          (set state.highlight
                                               (if (<= state.highlight 1)
                                                   (length panel.items)
                                                   (- state.highlight 1)))
                                          (changed))
                                        (and (= name :next)
                                             (> (length panel.items) 0))
                                        (do
                                          (set state.highlight
                                               (if (>= state.highlight
                                                       (length panel.items))
                                                   1
                                                   (+ state.highlight 1)))
                                          (changed))
                                        (= name :cycle)
                                        (do
                                          (table.insert session.view_ids
                                                        (table.remove session.view_ids
                                                                      1))
                                          (changed))
                                        (= name :cycle_previous)
                                        (do
                                          (table.insert session.view_ids 1
                                                        (table.remove session.view_ids))
                                          (changed))
                                        (= name :replace_view)
                                        {:consumed true :replace_view true}
                                        (= name :open_overlay)
                                        {:consumed true :open_overlay true}
                                        (and (= name :favorite)
                                             session.preference_scope
                                             (> panel.highlight 0))
                                        {:consumed true
                                         :favorite (. panel.items
                                                      panel.highlight :id)}
                                        (= name :cancel)
                                        (if (pop-narrow session db)
                                            {:consumed true :narrowed true}
                                            {:consumed true :cancelled true})
                                        (and (= name :accept)
                                             (> panel.highlight 0))
                                        (misa.choice_accept session
                                                            (. panel.items
                                                               panel.highlight)
                                                            db)
                                        {:consumed false})))})
          {:fx setup-fx})}
