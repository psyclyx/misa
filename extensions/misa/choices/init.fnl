(local definitions (require :misa.definitions))

;; Generic choice state and transitions shared by inline and overlay presentations.

(local key-context :choices)

(local right-home [:j :k :l :h ";"])
(local left-home [:f :d :s :a :g])
(local right-branch [:u :i :o :m :n])
(local left-branch [:r :e :w :t :q])
(local banks [right-home left-home right-home])
(local starters {})
(local continuations {})
(each [_ key (ipairs right-home)] (tset starters key true))
(each [_ key (ipairs right-branch)]
  (tset starters key true)
  (tset continuations key true))

;; Breadth-first leaves of an alternating-hand tree. Home keys terminate;
;; nearby keys continue. Leaves never prefix another leaf, and adding rows
;; cannot change an existing shortcut. Five short keys, 25 pairs, 125 triples.
(fn shortcut [rank]
  (var (depth count offset) (values 1 5 (- rank 1)))
  (while (>= offset count)
    (set offset (- offset count))
    (set count (* count 5))
    (set depth (+ depth 1)))
  (let [keys []]
    (for [level 1 depth]
      (let [divisor (^ 5 (- depth level))
            digit (+ 1 (math.floor (/ offset divisor)))]
        (set offset (% offset divisor))
        (let [right (= (% level 2) 1)
              alphabet (if (= level depth) (if right right-home left-home)
                           (if right right-branch left-branch))]
          (table.insert keys (. alphabet digit)))))
    (.. "alt+" (table.concat keys " "))))

(fn position-rank [bank slot]
  ;; Reserve the easiest single keys for the primary panel, then interleave
  ;; panels so secondary choices get short sequences too.
  (if (and (= bank 1) (<= slot 5)) slot
      (+ 6 (* 3 (- slot (if (= bank 1) 6 1))) (- bank 1))))

(local fallback {:alt+/ :replace_view
                 :alt+p :cycle_previous
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
                 :tab :complete})

(fn scalar? [v]
  (or (= (type v) :string) (= (type v) :number) (= (type v) :boolean)))

(fn item [source]
  (assert (= (type source) :table) "choice item must be a table")
  (let [value (if (not= source.value nil) source.value source.id)]
    (assert (scalar? value) "choice value must be scalar")
    (let [id (or source.id (and (= (type value) :string) value)
                 (tostring value))]
      (assert (and (= (type id) :string) (not= id ""))
              "choice id must be nonempty")
      (let [display source.display]
        (var (label description) nil)
        (if (= (type display) :string) (set label display)
            (= (type display) :table)
            (set (label description) (values display.label display.description)))
        (set (label description)
             (values (or label source.label id)
                     (or description source.description)))
        (assert (and (= (type label) :string)
                     (or (= description nil) (= (type description) :string)))
                "invalid choice display")
        (assert (or (= source.search nil) (= (type source.search) :string)
                    (= (type source.search) :table))
                "choice search must be a string or array")
        (var path (or source.path id))
        (when (= (type path) :table) (set path (table.concat path "/")))
        (assert (= (type path) :string) "choice path must be a string or array")
        {:browse_visible source.browse_visible
         : description
         : id
         :invocation source.invocation
         : label
         :narrow source.narrow
         : path
         :preview source.preview
         :search source.search
         :section source.section
         : value}))))

(fn items [source]
  (assert (= (type source) :table) "choice items must be an array")
  (let [result {}
        seen {}]
    (each [_ raw (ipairs source)]
      (let [value (item raw)]
        (assert (not (. seen value.id)) "choice ids must be unique")
        (tset seen value.id true)
        (tset result (+ (length result) 1) value)))
    result))

(fn search-text [value]
  (let [fields {}
        seen {}]
    (fn add [field]
      (when (and (= (type field) :string) (not= field "") (not (. seen field)))
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
  (if (= query "") source (if (and misa.fuzzy misa.fuzzy.choices)
                              (misa.fuzzy.choices source query search-text)
                              (let [result {}
                                    needle (query:lower)]
                                (each [_ value (ipairs source)]
                                  (when (or (= needle "")
                                            (: (: (search-text value) :lower)
                                               :find needle 1 true))
                                    (tset result (+ (length result) 1) value)))
                                result))))

(fn preference [db scope id]
  (let [scopes (and db.preferences db.preferences.scopes)]
    (and scopes (. scopes scope) (. scopes scope id))))

(fn compatible? [id session]
  (let [view (. (misa.catalog :choice-views) id)]
    (and view (or (not view.compatible) (view.compatible session)))))

(fn configured [purpose]
  (let [purposes (and (= (type misa.choices.config) :table) misa.choices
                      misa.choices.config misa.choices.config.purposes)]
    (var selected (or (and (= (type purposes) :table)
                           (or (. purposes purpose)
                               (. purposes (purpose:gsub "_" "-"))))
                      nil))
    (set selected (or selected [:all]))
    (assert (and (= (type selected) :table) (> (length selected) 0))
            "choice purpose must select at least one view")
    (let [result {}]
      (each [_ id (ipairs selected)]
        (assert (. (misa.catalog :choice-views) id)
                (.. "unknown choice view: " (tostring id)))
        (tset result (+ (length result) 1) id))
      result)))

(fn project-items [view session db]
  (if view.project (view.project session matches db)
      (let [source {}]
        (each [_ value (ipairs session.items)]
          (when (or (not view.include) (view.include value session db))
            (table.insert source value)))
        (when view.order
          (table.sort source (fn [a b] (view.order a b session db))))
        (matches source session.query))))

(local builtin-views {:all true :favorites true :frecency true :browse true})

(fn project [session slot db]
  (let [id (. session.view_ids slot)
        definition (and session.custom_views (. session.custom_views slot))
        view (or definition (assert (. (misa.catalog :choice-views) id)))
        source (if definition (matches definition.items session.query)
                   (. builtin-views id)
                   (misa.sub {:items session.items
                              :query session.query
                              :scope session.preference_scope
                              :preferences (and db.preferences
                                                db.preferences.scopes
                                                (. db.preferences.scopes
                                                   session.preference_scope))
                              :prior (and session.panels
                                          (. session.panels slot)
                                          (. session.panels slot :items))
                              :view id
                              :config misa.choices.config}
                             [:choices/builtin-items id slot])
                   (project-items view session db))
        state (or (. session.view_state id) {:highlight 0})
        previous (and (= state.query session.query) state.items
                      (. state.items state.highlight))]
    (var highlight (if (> (length source) 0) 1 0))
    (when previous
      (var found false)
      (each [index value (ipairs source) &until found]
        (when (= value.id previous.id)
          (set highlight index)
          (set found true))))
    {:filtered source : highlight : id :items source :title (or view.title id)}))

;; Projection equality is about preserving immutable output identity, not
;; validating state. Native transaction patches remain the validation boundary.
(fn same-projection? [left right]
  (if (= left right) true
      (or (not= (type left) :table) (not= (type right) :table)) false
      (and (accumulate [same true key value (pairs left) &until (not same)]
             (same-projection? value (. right key)))
           (accumulate [same true key _ (pairs right) &until (not same)]
             (not= (. left key) nil)))))

(fn refresh [session db]
  "Return a session with panels reconciled against current items and preferences."
  (let [panels {}
        states {}]
    (each [id state (pairs session.view_state)] (tset states id state))
    (for [slot 1 (length session.view_ids)]
      (let [projected (project session slot db)
            previous (and session.panels (. session.panels slot))
            panel (if (same-projection? previous projected) previous projected)]
        (tset panels slot panel)
        (let [state {:query session.query
                     :items panel.items
                     :highlight panel.highlight}]
          (tset states panel.id (if (same-projection? (. states panel.id) state)
                                    (. states panel.id)
                                    state)))))
    (if (and (same-projection? session.panels panels)
             (same-projection? session.view_state states))
        session
        (let [result {}]
          (each [key value (pairs session)] (tset result key value))
          (set result.panels panels)
          (set result.view_state states)
          result))))

(fn pop-utf8 [value]
  (var at (length value))
  (while (and (> at 0) (>= (value:byte at) 128) (< (value:byte at) 192))
    (set at (- at 1)))
  (value:sub 1 (math.max 0 (- at 1))))

(local positional-actions {})
(each [bank keys (ipairs banks)]
  (each [slot key (ipairs keys)]
    (when (= bank 1)
      (tset positional-actions (.. :alt+ key) (.. :option_ bank "_" slot)))))

(fn alt-text [event]
  (let [text (if (= event.kind :alt) event.text
                 (and (= event.kind :key) (= (type event.key) :string)) (event.key:match "^alt%+(.+)$"))]
    (when (= (type text) :string) (text:lower))))

(fn action [event]
  "Return the session transition for a named choice action."
  (let [alt (alt-text event)]
    (if (= (type event.action) :string) event.action (. continuations alt)
        :sequence misa.keybindings.action
        (misa.keybindings.action key-context event)
        (let [key (or (and (= event.kind :key) event.key)
                      (and (= event.kind :alt) (.. :alt+ (event.text:lower)))
                      event.kind)]
          (or (. fallback key) (. positional-actions key))))))

(fn needs-targets [session event]
  "Return whether this input needs positional choice targets."
  (let [name (action event)]
    (or (not= session.combo nil) (= name :sequence)
        (and (= (type name) :string) (not= (name:match "^option_%d+_%d+$") nil)))))

(fn hint [name]
  "Return the keyboard hint for a choice target."
  (let [configured (and misa.keybindings misa.keybindings.hint
                        (misa.keybindings.hint key-context name))
        (b n) (name:match "^option_(%d+)_(%d+)$")
        bank (tonumber b)
        slot (tonumber n)]
    (if configured configured (and bank slot (> slot 0) (. banks bank))
        (if (and (= bank 1) (<= slot 5) misa.keybindings misa.keybindings.hint)
            nil
            (shortcut (position-rank bank slot)))
        (not (and misa.keybindings misa.keybindings.hint))
        (let [keys (icollect [key value (pairs fallback)]
                     (when (= value name) key))]
          (table.sort keys)
          (. keys 1)))))

(fn key-text [event]
  (let [alt (alt-text event)]
    (or (and alt (.. "alt+" alt)) (and (= event.kind :key) event.key)
        (and (= event.kind :text) event.text))))

(fn sequence-targets [targets]
  (let [result {}]
    (each [name item (pairs (or targets {}))]
      (let [key (hint name)]
        (when key
          (assert (not (. result key))
                  "visible choice shortcuts must be unique")
          (tset result key item))))
    result))

(fn sequence-input [session event]
  (let [key (if session.combo (or (alt-text event) (key-text event))
                (key-text event))
        pending session.combo
        cleared (if pending
                    (misa.patch session
                                {:combo misa.delete :combo_targets misa.delete})
                    session)
        sequence (and key (if pending (.. pending " " key) key))
        targets (sequence-targets event.targets)
        target (and sequence (. targets sequence))
        captured session.combo_targets]
    (if (and target (or (not pending) (= (. captured sequence) target.id)))
        {:session cleared :accepted target :consumed true :positional true}
        (let [remaining {}]
          (when sequence
            (each [shortcut item (pairs targets)]
              (when (and (= (shortcut:sub 1 (+ (length sequence) 1))
                            (.. sequence " "))
                         (or (not pending) (= (. captured shortcut) item.id)))
                (tset remaining shortcut item.id))))
          (if (next remaining)
              {:session (misa.patch session
                                    {:combo sequence
                                     :combo_targets (misa.replace remaining)})
               :consumed true}
              pending
              {:session cleared :consumed true}
              nil)))))

(fn first [panel room]
  "Return the first visible item index for a choice panel."
  (if (or (<= room 0) (<= (length panel.items) room))
      1
      (math.max 1
                (math.min (- panel.highlight (math.floor (/ room 2)))
                          (+ (- (length panel.items) room) 1)))))

(fn source-spec [id context db]
  (let [source (assert (. (misa.catalog :choice-sources) id)
                       (.. "unknown choice source: " (tostring id)))
        spec (or (source.items context db) {})]
    (if spec.items spec {:items spec})))

(fn apply-spec [session spec db]
  (let [purpose (or spec.purpose session.purpose)]
    (refresh (misa.patch session
                         {:title (or spec.title session.title)
                          : purpose
                          :items (misa.replace (items (or spec.items {})))
                          :query (or spec.query "")
                          :input_prefix (or spec.input_prefix "")
                          :selected (misa.replace (when (not= spec.selected
                                                              misa.json-null)
                                                    spec.selected))
                          :preference_scope (misa.replace spec.preference_scope)
                          :view_state (misa.replace {})
                          :view_ids (misa.replace (or spec.views
                                                      (configured purpose)))
                          :custom_views misa.delete}) db)))

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
  (let [frame {}
        stack {}]
    (each [_ key (ipairs frame-keys)] (tset frame key (. session key)))
    (each [index value (ipairs session.stack)] (tset stack index value))
    (tset stack (+ (length stack) 1) frame)
    (let [spec (or (and narrow.source
                        (source-spec narrow.source narrow.context db))
                   narrow)]
      {:session (apply-spec (misa.patch session {:stack (misa.replace stack)})
                            spec db)
       :consumed true
       :narrowed true})))

(fn pop-narrow [session db]
  (let [frame (. session.stack (length session.stack))]
    (if (not frame)
        nil
        (let [patch {}
              stack {}]
          (for [index 1 (- (length session.stack) 1)]
            (tset stack index (. session.stack index)))
          (each [_ key (ipairs frame-keys)]
            (tset patch key (misa.replace (. frame key))))
          (tset patch :stack (misa.replace stack))
          (refresh (misa.patch session patch) db)))))

(fn new [spec db]
  "Create a choice session from an explicit specification and database."
  (assert (and (= (type spec) :table) (= (type spec.title) :string)
               (not= spec.title ""))
          "choice session requires a title")
  (let [session (apply-spec {:stack {}} spec db)]
    (if spec.view_definitions
        (let [ids {}
              custom {}]
          (each [index definition (ipairs spec.view_definitions)]
            (tset ids index (assert definition.id))
            (tset custom index
                  {:id definition.id
                   :items (items (or definition.items {}))
                   :title definition.title}))
          (refresh (misa.patch session
                               {:view_ids (misa.replace ids)
                                :custom_views (misa.replace custom)})
                   db))
        session)))

(fn accept [session value db]
  "Return the selected choice and its acceptance transition."
  (if value.narrow (push-narrow session value.narrow db)
      {: session :accepted value :consumed true}))

(fn changed [session patch db]
  {:session (refresh (misa.patch session patch) db) :consumed true})

(fn move [session direction _db]
  ;; Moving only changes focus. Build immutable projection records here; the
  ;; owning transaction validates the changed state once at the commit boundary.
  (let [panel (. session.panels 1)
        count (length panel.items)]
    (if (= count 0) {: session :consumed false} (= count 1)
        {: session :consumed true}
        (let [highlight (+ (% (+ (- panel.highlight 1) direction) count) 1)
              focused {}
              panels {}
              states {}
              next-state {}
              next-session {}]
          (each [key value (pairs panel)] (tset focused key value))
          (set focused.highlight highlight)
          (each [bank value (ipairs session.panels)]
            (tset panels bank (if (= bank 1) focused value)))
          (each [key value (pairs (. session.view_state panel.id))]
            (tset next-state key value))
          (set next-state.highlight highlight)
          (each [key value (pairs session.view_state)] (tset states key value))
          (tset states panel.id next-state)
          (each [key value (pairs session)] (tset next-session key value))
          (set next-session.panels panels)
          (set next-session.view_state states)
          {:session next-session :consumed true}))))

(fn cycle [session direction db]
  (let [ids {}
        custom {}
        count (length session.view_ids)]
    (for [slot 1 count]
      (let [source (+ (% (+ (- slot 1) direction) count) 1)]
        (tset ids slot (. session.view_ids source))
        (when session.custom_views
          (tset custom slot (. session.custom_views source)))))
    (changed session
             {:view_ids (misa.replace ids)
              :custom_views (misa.replace (when session.custom_views
                                            custom))} db)))

(fn accept-2 [session _ db]
  (let [panel (. session.panels 1)]
    (if (> panel.highlight 0)
        (accept session (. panel.items panel.highlight) db)
        {: session :consumed false})))

(fn cancel [session _ db]
  (let [parent (pop-narrow session db)]
    {:session (or parent session)
     :consumed true
     :cancelled (= parent nil)
     :narrowed (not= parent nil)}))

(fn favorite [session]
  (let [panel (. session.panels 1)]
    (if (and session.preference_scope (> panel.highlight 0))
        {: session
         :consumed true
         :favorite (. panel.items panel.highlight :id)}
        {: session :consumed false})))

(fn backspace [session _ db]
  (if (not= session.query "")
      (changed session {:query (pop-utf8 session.query)} db)
      (let [parent (pop-narrow session db)]
        {:session (or parent session)
         :consumed (not= parent nil)
         :narrowed (not= parent nil)})))

(fn text-2 [session event db]
  (if (= (type event.text) :string)
      (changed session {:query (.. session.query event.text)} db)
      {: session :consumed false}))

(local inputs {:text text-2
               :backspace backspace
               :previous (fn [session _ db] (move session (- 1) db))
               :next (fn [session _ db] (move session 1 db))
               :cycle (fn [session _ db] (cycle session 1 db))
               :cycle_previous (fn [session _ db] (cycle session (- 1) db))
               :replace_view (fn [session]
                               {: session :consumed true :replace_view true})
               :open_overlay (fn [session]
                               {: session :consumed true :open_overlay true})
               :favorite favorite
               :cancel cancel
               :accept accept-2})

(fn complete [session db]
  (let [panel (. session.panels 1)]
    (var common nil)
    (each [_ value (ipairs panel.items)]
      (let [prefix (or session.input_prefix "")
            path (if (and (not= prefix "")
                          (= (value.path:sub 1 (length prefix)) prefix))
                     (value.path:sub (+ (length prefix) 1))
                     value.path)]
        (if (= common nil) (set common path)
            (do
              (var n 0)
              (while (and (< n (math.min (length common) (length path)))
                          (= (common:sub (+ n 1) (+ n 1))
                             (path:sub (+ n 1) (+ n 1))))
                (set n (+ n 1)))
              ;; Do not leave a partial UTF-8 character at a bytewise branch point.
              (while (and (> n 0) (< n (length path))
                          (>= (path:byte (+ n 1)) 128)
                          (< (path:byte (+ n 1)) 192))
                (set n (- n 1)))
              (set common (common:sub 1 n))))))
    ;; Extend the common prefix first; another Tab completes the focused choice.
    ;; Completion only edits the query. Enter owns acceptance and execution.
    (if (and common (> (length common) (length session.query)))
        (changed session {:query common} db)
        (let [value (. panel.items panel.highlight)
              prefix (or session.input_prefix "")
              path (and value (if (and (not= prefix "")
                                       (= (value.path:sub 1 (length prefix))
                                          prefix))
                                  (value.path:sub (+ (length prefix) 1))
                                  value.path))]
          (if (and path (not= path session.query))
              (changed session {:query path} db)
              {: session :consumed true})))))

(fn input [previous event db]
  "Translate terminal input into a choice session transition."
  (let [session (refresh previous db)
        sequence (sequence-input session event)]
    (if sequence
        (if sequence.accepted
            (misa.patch (accept sequence.session sequence.accepted db)
                        {:positional true})
            sequence)
        (let [target (and event.targets (. event.targets event.action))
              name (if (or (= event.kind :text) (= event.kind :backspace))
                       event.kind
                       event.action)
              handler (. (misa.catalog :choice-inputs) name)]
          (if target (misa.patch (accept session target db) {:positional true})
              (= name :complete) (complete session db)
              handler (handler session event db)
              {: session :consumed false})))))

(fn row [value focused selected hotkey]
  (var description value.description)
  (when (or (= description value.label) (= description value.id)
            (= description (tostring value.value)))
    (set description nil))
  {:active focused
   : description
   : hotkey
   :id value.id
   :label value.label
   :marker (or (and focused ">") (and selected "✓") " ")
   :path value.path
   :preview value.preview
   :section value.section
   : selected
   :value value.value})

(fn compute-choices-builtin-items [inputs]
  (let [scope (. inputs 3)
        session {:items (. inputs 1)
                 :query (. inputs 2)
                 :preference_scope scope}
        db {:preferences {:scopes {}}}]
    (when scope
      (tset db.preferences.scopes scope (. inputs 4)))
    (let [result (project-items (. (misa.catalog :choice-views) (. inputs 6))
                                session db)
          prior (. inputs 5)]
      (if (same-projection? prior result)
          prior
          result))))

(fn choices-pending [db]
  "Return the active keyboard sequence, if a choice session has one."
  (let [session (or (and db.picker db.picker.session)
                    (and db.editor db.editor.choice))]
    (and session session.combo)))

(fn route-terminal-input [db event]
  (let [session (or (and db.picker db.picker.session)
                    (and db.editor db.editor.choice))]
    (when (and session
               (or session.combo
                   (and (alt-text event) (. starters (alt-text event)))))
      (misa.patch event {:type (if db.picker
                                   :picker/input
                                   :terminal/input)}))))

(fn route-ui-action [db event]
  (when (and (or db.picker (and db.editor db.editor.choice))
             (= (type event.action) :string)
             (event.action:match "^choices%.option_%d+_%d+$"))
    (if (misa.choices.pending db)
        {:type :choices/ignored}
        {:type :choices/dispatch :action (event.action:sub 9)})))

(fn on-choices-dispatch [db event]
  (if (and (not db.picker) (not (and db.editor db.editor.choice)))
      {:fx [{:type :terminal/read}]}
      {:fx [{:event {:action event.action
                     :kind :choice_action
                     :type (or (and db.picker :picker/input) :terminal/input)}
             :type :dispatch}]}))

(fn choices-source [id context-value db]
  "Return command choice items for the current query."
  (source-spec id context-value db))

(fn choices-set-items [session source-items db]
  "Return a refreshed session containing the supplied items."
  (refresh (misa.patch session {:items (misa.replace (items source-items))}) db))

(fn choices-registered-views [session]
  "List compatible registered views in stable ID order."
  (let [result {}]
    (each [id view (pairs (misa.catalog :choice-views))]
      (when (compatible? id session)
        (tset result (+ (length result) 1)
              {:display (or view.title id) : id :search id :value id})))
    (table.sort result (fn [a b] (< a.id b.id)))
    result))

(fn choices-projected-rows [session hotkeys]
  "Build presentation rows for every choice panel."
  (let [result {}]
    (each [bank panel (ipairs session.panels)]
      (let [rows {}]
        (each [index value (ipairs panel.items)]
          (tset rows index
                (row value (and (= bank 1) (= index panel.highlight))
                     (= value.value session.selected)
                     (and hotkeys (. hotkeys bank) (. hotkeys bank index)))))
        (tset result bank {:id panel.id : rows :title panel.title})))
    result))

(fn choices-rows [previous db hotkeys]
  "Return presentation rows for a choice panel."
  (misa.choices.projected-rows (refresh previous db) hotkeys))

(fn choices-hotkeys [session visible room]
  "Return keyboard targets for the visible choice rows."
  (let [result {}]
    (for [bank 1 (math.min (or visible 0) (length session.panels)
                           (length banks))]
      (tset result bank {})
      (let [panel (. session.panels bank)
            start (first panel room)]
        (for [slot 1 room]
          (when (. panel.items (- (+ start slot) 1))
            (tset (. result bank) (- (+ start slot) 1)
                  (hint (.. :option_ bank "_" slot)))))))
    result))

(fn choices-positional [session name visible room]
  "Return positional targets for the supplied panel geometry."
  (var result nil)
  (for [bank 1 (math.min (or visible 0) (length session.panels) (length banks))
        &until result]
    (for [slot 1 (or room 0) &until result]
      (when (= name (.. :option_ bank "_" slot))
        (set result
             (. session.panels bank :items
                (- (+ (first (. session.panels bank) room) slot) 1))))))
  result)

(fn favorite? [value session db]
  (let [p (preference db session.preference_scope value.id)]
    (and p (= p.favorite true))))

(fn recent-before? [a b session db]
  (let [pa (or (preference db session.preference_scope a.id) {})
        pb (or (preference db session.preference_scope b.id) {})]
    (> (+ (* (or pa.last 0) 1000000) (or pa.uses 0))
       (+ (* (or pb.last 0) 1000000) (or pb.uses 0)))))

(fn used? [value session db]
  (let [p (preference db session.preference_scope value.id)]
    (and p (> (or p.uses 0) 0))))

(fn browse [session match-items db]
  (let [all (match-items session.items session.query)]
    (if (not= session.query "") all
        (let [curated (accumulate [hidden false _ value (ipairs all)]
                        (or hidden (= value.browse_visible false)))
              recent {}]
          (each [_ value (ipairs all)]
            (let [p (preference db session.preference_scope value.id)]
              (when (and p (> (or p.uses 0) 0))
                (tset recent (+ (length recent) 1) value))))
          (table.sort recent
                      (fn [a b]
                        (let [pa (preference db session.preference_scope a.id)
                              pb (preference db session.preference_scope b.id)]
                          (if (= pa.last pb.last)
                              (< a.id b.id)
                              (> (or pa.last 0) (or pb.last 0))))))
          (let [result {}
                seen {}
                limit (math.max 0
                                (math.floor (or (tonumber misa.choices.config.recent_limit)
                                                3)))]
            (for [index 1 (math.min (length recent) limit
                                    (math.max 0 (- (length all) 1)))]
              (let [value (misa.patch (. recent index) {:section :Recent})]
                (tset result (+ (length result) 1) value)
                (tset seen value.id true)))
            (each [_ value (ipairs all)]
              (when (and (not (. seen value.id))
                         (not= value.browse_visible false))
                (table.insert result
                              (misa.patch value
                                          {:section (if curated
                                                        :Suggested
                                                        :All)}))))
            result)))))

(fn session-open? [db]
  (or (not= db.picker nil) (and db.editor (not= db.editor.choice nil)) false))

(fn choices-replace-view [session id db]
  "Return a session with its primary view replaced by a compatible view."
  (assert (compatible? id session) "incompatible choice view")
  (let [ids {}
        custom {}]
    (each [index value (ipairs session.view_ids)]
      (tset ids index (if (= index 1) id value)))
    (when session.custom_views
      (each [index value (ipairs session.custom_views)]
        (tset custom index (if (= index 1)
                               false
                               value))))
    (refresh (misa.patch session
                         {:view_ids (misa.replace ids)
                          :custom_views (misa.replace (when session.custom_views
                                                        custom))})
             db)))

(fn choice-sources [_ source]
  (assert (and (= (type source) :table) (= (type source.items) :function))
          "choice source requires items"))

(fn choice-views [_ view]
  (assert (= (type view) :table) "choice view must be a table"))

(fn choice-inputs [_ handler]
  (assert (= (type handler) :function) "choice input must be a function"))

(fn build [context]
  "Build the declarations for choices."
  (let [declarations []]
    (table.insert declarations {:catalog :services
                                :id :choices.config
                                :value (or (and (= (type context.config) :table)
                                                (= (type context.config.choices)
                                                   :table)
                                                context.config.choices)
                                           {})})
    ;; Only built-in views have this closed dependency contract. Extension
    ;; views keep receiving the complete current session/database each call.
    ;; Including prior items lets the cache adopt canonical committed arrays
    ;; after native validation, then reuse them while only focus changes.
    (table.insert declarations
                  (let [definition {:id :choices/builtin-items
                                    :inputs [[:db/path :items]
                                             [:db/path :query]
                                             [:db/path :scope]
                                             [:db/path :preferences]
                                             [:db/path :prior]
                                             [:db/path :view]
                                             [:db/path :config]]
                                    :compute compute-choices-builtin-items}]
                    {:catalog :subscriptions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  {:catalog :services
                   :id :choices.pending
                   :value choices-pending})
    (table.insert declarations
                  {:catalog :services
                   :id :choices.needs-targets?
                   :value needs-targets})
    (table.insert declarations
                  (let [definition {:id :choices/sequence
                                    :event :terminal/input
                                    :priority 950
                                    :context [:db/path]
                                    :resolve route-terminal-input}]
                    {:catalog :routes :id (. definition :id) :value definition}))
    (table.insert declarations
                  (let [definition {:id :choices/visible-action
                                    :event :ui/action
                                    :priority 800
                                    :context [:db/path]
                                    :resolve route-ui-action}]
                    {:catalog :routes :id (. definition :id) :value definition}))
    (table.insert declarations
                  {:catalog :events
                   :value {:event :choices/ignored
                           :handler (fn []
                                      {:fx [{:type :terminal/read}]})}})
    (let [key-defaults {:accept [:enter]
                        :complete [:tab]
                        :cancel [:escape :ctrl_c :ctrl_d :eof]
                        :cycle [:arrow_right]
                        :cycle_previous [:arrow_left :alt+p]
                        :favorite [:alt+v]
                        :next [:arrow_down]
                        :open_overlay []
                        :previous [:arrow_up]
                        :replace_view [:alt+/]}]
      (each [name keys (pairs key-defaults)]
        (table.insert declarations
                      (let [definition {:action name
                                        :context key-context
                                        :default keys}]
                        {:catalog :keybindings
                         :id (.. (. definition :context) "/"
                                 (. definition :action))
                         :value definition})))
      (each [bank keys (ipairs banks)]
        (for [slot 1 9]
          (table.insert declarations
                        (let [definition {:action (.. :option_ bank "_" slot)
                                          :context key-context
                                          :default (if (and (= bank 1)
                                                            (. keys slot))
                                                       [(.. :alt+ (. keys slot))]
                                                       [])}]
                          {:catalog :keybindings
                           :id (.. (. definition :context) "/"
                                   (. definition :action))
                           :value definition}))))
      (let [labels {:accept "Accept choice"
                    :complete "Complete to branch"
                    :cancel "Cancel choices / back"
                    :cycle "Next choice view"
                    :cycle_previous "Previous choice view"
                    :favorite "Toggle favorite choice"
                    :next "Next choice"
                    :open_overlay "Expand inline choices"
                    :previous "Previous choice"
                    :replace_view "Replace choice view"}]
        (each [name label (pairs labels)]
          (let [action-name name]
            (fn action-available? [db]
              (let [session (or (and db.picker db.picker.session)
                                (and db.editor db.editor.choice))]
                (and (not= session nil)
                     (or (not= action-name :favorite)
                         (not= session.preference_scope nil))
                     (or (not= action-name :open_overlay) (= db.picker nil)))))

            (table.insert declarations
                          (let [definition {:available action-available?
                                            :binding {:action name
                                                      :context key-context}
                                            :event {:action name
                                                    :type :choices/dispatch}
                                            :id (.. :choices. name)
                                            : label}]
                            {:catalog :actions
                             :id (. definition :id)
                             :value definition}))))
        ;; Positional targets use the same geometry resolver for keyboard and pointer.
        ;; A stale or hidden button therefore cannot choose a different hidden row.
        (each [bank keys (ipairs banks)]
          (for [slot 1 9]
            (let [name (.. :option_ bank "_" slot)]
              (table.insert declarations
                            (let [definition {:available session-open?
                                              :binding {:action name
                                                        :context key-context}
                                              :event {:action name
                                                      :type :choices/dispatch}
                                              :id (.. :choices. name)
                                              :label (.. "Choose visible item "
                                                         bank ":" slot)}]
                              {:catalog :actions
                               :id (. definition :id)
                               :value definition})))))
        (table.insert declarations
                      {:catalog :events
                       :value {:event :choices/dispatch
                               :handler on-choices-dispatch}})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.source
                       :value choices-source})
        (table.insert declarations
                      {:catalog :choice-views :id :all :value {:title :All}})
        (table.insert declarations
                      {:catalog :choice-views
                       :id :favorites
                       :value {:include favorite? :title :Favorites}})
        (table.insert declarations
                      {:catalog :choice-views
                       :id :frecency
                       :value {:include used?
                               :order recent-before?
                               :title :Recent}})
        ;; Grouping is a view policy. The layout treats section names as ordinary
        ;; row metadata, so extensions can compose other grouped lists the same way.
        (table.insert declarations
                      {:catalog :choice-views
                       :id :browse
                       :value {:project browse :title :Browse}})
        (table.insert declarations
                      {:catalog :services :id :choices.session :value new})
        (table.insert declarations
                      {:catalog :services :id :choices.refresh :value refresh})
        (table.insert declarations
                      {:catalog :services :id :choices.action :value action})
        (table.insert declarations
                      {:catalog :services :id :choices.hint :value hint})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.first-index
                       :value first})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.set-items
                       :value choices-set-items})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.replace-view
                       :value choices-replace-view})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.registered-views
                       :value choices-registered-views})
        (table.insert declarations
                      {:catalog :services :id :choices.accept :value accept})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.projected-rows
                       :value choices-projected-rows})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.rows
                       :value choices-rows})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.hotkeys
                       :value choices-hotkeys})
        (table.insert declarations
                      {:catalog :services
                       :id :choices.positional
                       :value choices-positional})
        (table.insert declarations
                      {:catalog :services :id :choices.input :value input})
        (definitions.build :choices
          declarations
          {:choice-inputs inputs
           :validators {:choice-inputs choice-inputs
                        :choice-views choice-views
                        :choice-sources choice-sources}})))))

{: build}
