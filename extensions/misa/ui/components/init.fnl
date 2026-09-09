(local definitions (require :misa.definitions))

;; Semantic visual component registry. Implementations are immutable registration
;; data; every role selection lives in transactional application state.

(fn shallow [source]
  (let [result {}]
    (each [key value (pairs source)] (tset result key value))
    result))

(fn same-fields? [a b]
  (if (= a b) true
      (or (not a) (not b)) false
      (do
        (each [key value (pairs a)]
          (when (not= value (. b key)) (lua "return false")))
        (each [key value (pairs b)]
          (when (not= value (. a key)) (lua "return false")))
        true)))

(fn hover-targets [rendered]
  (let [actions {}
        links {}]
    (each [_ line (ipairs (or rendered.lines []))]
      (each [_ span (ipairs (or line.spans []))]
        (if span.action (tset actions span.action true)
            span.link (tset links span.link true))))
    (values actions links)))

;; Validate semantic output once, before it enters composition or a cache. The
;; terminal still validates the final native frame, including payload encodings.
(fn integer [value minimum maximum]
  (and (= (type value) :number) (= value (math.floor value)) (>= value minimum)
       (<= value maximum)))

(fn array [value label]
  (assert (= (type value) :table) (.. label " must be an array"))
  (each [key _ (pairs value)]
    (assert (integer key 1 (length value)) (.. label " must be a dense array")))
  value)

(fn printable [text]
  (assert (and (= (type text) :string) (not (text:find "[%z\1-\31\127]")))
          "span text must be printable text on one line"))

(fn validate [rendered]
  (assert (= (type rendered) :table) "component render must return a table")
  (each [_ line (ipairs (array rendered.lines :lines))]
    (assert (= (type line) :table) "line must be a table")
    (each [_ span (ipairs (array line.spans :spans))]
      (assert (= (type span) :table) "span must be a table")
      (printable span.text)
      (each [_ field (ipairs [:action :link])]
        (when (. span field)
          (assert (= (type (. span field)) :string) (.. field " must be text"))))
      (when span.animation
        (assert (= (type span.animation) :table) "animation must be a table")
        (each [_ frame (ipairs (array span.animation.frames :frames))]
          (assert (= (type frame) :table) "animation frame must be a table")
          (each [key _ (pairs frame)]
            (assert (or (= key :text) (= key :style))
                    "invalid animation frame field"))
          (when frame.text (printable frame.text)))
        (assert (and (= (type span.animation.id) :string)
                     (not= span.animation.id "")
                     (integer span.animation.interval_ms 10 60000)
                     (integer (length span.animation.frames) 1 64)
                     (integer (or span.animation.phase 0) 0
                              (- (length span.animation.frames) 1)))
                "invalid animation timing or identity")))
    (when line.image
      (let [image line.image]
        (assert (= (type image) :table) "image must be a table")
        (each [_ pair (ipairs [[:id 4294967295]
                               [:width 480]
                               [:height 320]
                               [:columns 65535]
                               [:rows 65535]])]
          (assert (integer (. image (. pair 1)) 1 (. pair 2))
                  "invalid image geometry"))
        (when image.column
          (assert (integer image.column 1 65535) "invalid image column"))
        (assert (and (= image.format :rgba) (= (type image.data) :string))
                "invalid image payload"))))
  (when rendered.cursor
    (let [cursor rendered.cursor]
      (assert (and (= (type cursor) :table)
                   (integer cursor.row 1 (length rendered.lines)))
              "invalid cursor row")
      (let [spans (. rendered.lines cursor.row :spans)]
        (assert (and (= cursor.column nil)
                     (or (= cursor.shape nil) (= cursor.shape :bar)
                         (= cursor.shape :block)))
                "invalid cursor shape or column")
        (var bytes 0)
        (each [_ span (ipairs spans)]
          (set bytes (+ bytes (length span.text))))
        (assert (integer cursor.byte 0 bytes) "invalid cursor byte")
        (var start 0)
        (each [_ span (ipairs spans)]
          (when (and (>= cursor.byte start)
                     (< cursor.byte (+ start (length span.text))))
            (let [byte (span.text:byte (+ (- cursor.byte start) 1))]
              (assert (or (< byte 128) (>= byte 192))
                      "cursor byte splits a UTF-8 character")))
          (set start (+ start (length span.text)))))))
  rendered)

(fn failure [role context diagnostic]
  (let [message (.. "Component " role " failed")
        requested (and (= (type context) :table) context.columns)
        columns (if (integer requested 0 65535) requested (length message))
        text (if (and misa.layout misa.layout.clip)
                 (misa.layout.clip message (math.max 0 columns))
                 (: (message:gsub "[\128-\255]" "?") :sub 1
                    (math.max 0 columns)))]
    {:component_error {: role :detail (tostring diagnostic)}
     :lines [{:content false
              :component_error {: role :detail (tostring diagnostic)}
              :spans [{: text}]}]}))

(fn build [context]
  "Build the declarations for components."
  (let [declarations []]
    (var config (or (and (= (type context.config) :table)
                         context.config.components) nil))
    (set config (or (and (= (type config) :table) config) {}))
    (let [configured (or (and (= (type config.roles) :table) config.roles)
                         config)]
      (table.insert declarations
                    {:catalog :services
                     :id :components.lookup
                     :value (fn [db role]
                              "Resolve the implementation selected for a component role."
                              (assert (= (type db) :table)
                                      "component resolution requires db")
                              (assert (and (= (type role) :string)
                                           (not= role ""))
                                      "component role must be nonempty")
                              (let [state (assert db.components
                                                  "component state is not initialized")
                                    id (or (. state.roles role)
                                           (.. :default. role))]
                                (assert (and (= (type id) :string) (not= id ""))
                                        (.. "no component configured for role: "
                                            role))
                                (assert (. (misa.catalog :components) id)
                                        (.. "unknown component for " role ": "
                                            id))))})
      (table.insert declarations
                    {:catalog :services
                     :id :components.resolve
                     :value (fn [db rendered render-context]
                              "Resolve semantic component styles without mutating the source."
                              (assert (= (type rendered) :table)
                                      "component render must return a table")
                              ;; Components emit semantic tokens (or ordered token lists) and know
                              ;; nothing about terminal colors. Resolution composes one native record.
                              (assert misa.themes.style
                                      "components requires the themes service")
                              ;; Resolve into output records; component-owned semantic caches remain reusable.
                              (let [result (shallow rendered)]
                                (when rendered.lines
                                  (set result.lines [])
                                  (each [_ line (ipairs rendered.lines)]
                                    (let [resolved-line (shallow line)]
                                      (set resolved-line.spans [])
                                      (table.insert result.lines resolved-line)
                                      (let [surface (or line.surface
                                                        rendered.surface)
                                            base (if surface
                                                     (misa.themes.style db
                                                                        surface)
                                                     {})
                                            texts []]
                                        (each [_ span (ipairs (or line.spans []))]
                                          (let [resolved-span (shallow span)
                                                style (shallow base)
                                                disabled (and span.action
                                                              misa.choices
                                                              misa.choices.pending
                                                              (misa.choices.pending db))]
                                            (each [key value (pairs (misa.themes.style db
                                                                                       (or span.style
                                                                                           :plain)))]
                                              (tset style key value))
                                            (when (and (not disabled)
                                                       (or (and span.action
                                                                (= span.action
                                                                   db.hover_action))
                                                           (and (not span.action)
                                                                span.link
                                                                (= span.link
                                                                   db.hover_link))))
                                              (each [key value (pairs (misa.themes.style db
                                                                                         :hover))]
                                                (tset style key value)))
                                            (when disabled
                                              (when (not span.sequence_progress)
                                                (each [key value (pairs (misa.themes.style db
                                                                                           :disabled))]
                                                  (tset style key value)))
                                              (set resolved-span.action nil))
                                            (set resolved-span.style style)
                                            (when span.animation
                                              (var animation nil)
                                              (each [index frame (ipairs (or span.animation.frames
                                                                             []))]
                                                (when frame.style
                                                  (when (not animation)
                                                    (set animation
                                                         (shallow span.animation))
                                                    (set animation.frames
                                                         (shallow span.animation.frames))
                                                    (set resolved-span.animation
                                                         animation))
                                                  (let [resolved-frame (shallow frame)
                                                        frame-style (shallow style)]
                                                    (each [key value (pairs (misa.themes.style db
                                                                                               frame.style))]
                                                      (tset frame-style key
                                                            value))
                                                    (when (and disabled
                                                               (not span.sequence_progress))
                                                      (each [key value (pairs (misa.themes.style db
                                                                                                 :disabled))]
                                                        (tset frame-style key
                                                              value)))
                                                    (set resolved-frame.style
                                                         frame-style)
                                                    (tset animation.frames
                                                          index resolved-frame)))))
                                            (table.insert resolved-line.spans
                                                          resolved-span)
                                            (table.insert texts
                                                          (or span.text ""))))
                                        ;; A surface belongs to the box, including unused row cells.
                                        (when (and surface render-context
                                                   render-context.columns
                                                   misa.layout)
                                          (let [padding (math.max 0
                                                                  (- render-context.columns
                                                                     (misa.layout.width (table.concat texts))))]
                                            (when (> padding 0)
                                              (table.insert resolved-line.spans
                                                            {:style base
                                                             :text (string.rep " "
                                                                               padding)}))))))))
                                result))})
      ;; This implementation capability keeps child failures inside their
      ;; boundary while allowing parents to lay out measured child views.
      (var render-safe nil)

      (fn child-context [db context]
        (let [next (shallow (or context {}))]
          (set next.render_child
               (fn [role model child]
                 (let [context (or child next)]
                   (render-safe db role model context))))
          next))

      (set render-safe
           (fn [db role model context cache]
             (let [(ok rendered next-cache) (pcall (fn []
                                                     (let [component (misa.components.lookup db
                                                                                             role)
                                                           (view next-cache) (component.render model
                                                                                               (if component.compose
                                                                                                   (child-context db
                                                                                                                  context)
                                                                                                   (or context
                                                                                                       {}))
                                                                                               cache)]
                                                       (values (validate view)
                                                               next-cache))))]
               (if ok (values rendered next-cache)
                   (values (failure role context rendered) nil)))))

      (fn resolve-safe [db role source context]
        (let [(ok view) (pcall misa.components.resolve db source context)]
          (if ok view (failure role context view))))

      (table.insert declarations
                    {:catalog :services
                     :id :components.render
                     :value (fn [db role model render-context]
                              "Render a selected component and contain presentation failures."
                              (let [rendered (render-safe db role model
                                                          render-context)]
                                (resolve-safe db role rendered render-context)))})
      (table.insert declarations
                    (let [definition {:id :components/projection
                                      :inputs [[:db/path :db]
                                               [:db/path :items]
                                               [:db/path :context]]
                                      :compute (fn [inputs _ previous]
                                                 (let [db (. inputs 1)
                                                       items (. inputs 2)
                                                       render-context (. inputs
                                                                         3)
                                                       theme (misa.themes.lookup db)
                                                       entries {}
                                                       views []]
                                                   (each [_ item (ipairs items)]
                                                     (assert (and (= (type item.id)
                                                                     :string)
                                                                  (not= item.id
                                                                        "")
                                                                  (not (. entries
                                                                          item.id)))
                                                             "component collection requires unique nonempty ids")
                                                     (let [old (and previous
                                                                    (. previous.entries
                                                                       item.id))
                                                           (found selected) (pcall misa.components.lookup
                                                                                   db
                                                                                   item.role)
                                                           component (and found
                                                                          selected)
                                                           same (and old
                                                                     (or (and component
                                                                              (not component.compose))
                                                                         (= db.components
                                                                            old.components))
                                                                     (= component
                                                                        old.component)
                                                                     (same-fields? item.model
                                                                                   old.model)
                                                                     (same-fields? render-context
                                                                                   old.context))
                                                           (source cache) (if same
                                                                              (values old.source
                                                                                      old.cache)
                                                                              (render-safe db
                                                                                           item.role
                                                                                           item.model
                                                                                           render-context
                                                                                           (and old
                                                                                                (= component
                                                                                                   old.component)
                                                                                                old.cache)))
                                                           (actions links) (if same
                                                                               (values old.actions
                                                                                       old.links)
                                                                               (hover-targets source))
                                                           hover-action (and db.hover_action
                                                                             (. actions
                                                                                db.hover_action)
                                                                             db.hover_action)
                                                           hover-link (and db.hover_link
                                                                           (. links
                                                                              db.hover_link)
                                                                           db.hover_link)
                                                           entry (if (and same
                                                                          (= theme
                                                                             old.theme)
                                                                          (= hover-action
                                                                             old.hover_action)
                                                                          (= hover-link
                                                                             old.hover_link)
                                                                          (= (and misa.choices
                                                                                  misa.choices.pending
                                                                                  (misa.choices.pending db))
                                                                             old.choice_pending))
                                                                     old
                                                                     {: component
                                                                      :components db.components
                                                                      :model item.model
                                                                      :context render-context
                                                                      : source
                                                                      : cache
                                                                      : actions
                                                                      : links
                                                                      : theme
                                                                      :hover_action hover-action
                                                                      :hover_link hover-link
                                                                      :choice_pending (and misa.choices
                                                                                           misa.choices.pending
                                                                                           (misa.choices.pending db))
                                                                      :view (resolve-safe db
                                                                                          item.role
                                                                                          source
                                                                                          render-context)})]
                                                       (tset entries item.id
                                                             entry)
                                                       (table.insert views
                                                                     entry.view)))
                                                   {: entries : views}))}]
                      {:catalog :subscriptions
                       :id (. definition :id)
                       :value definition}))
      (table.insert declarations
                    {:catalog :services
                     :id :components.project
                     :value (fn [db id items render-context]
                              "Project an ordered component collection with reusable per-item caches."
                              (assert (and (= (type id) :string) (not= id ""))
                                      "component collection requires an owner id")
                              (misa.sub {: db
                                         : items
                                         :context (or render-context {})}
                                        [:components/projection id]))})
      (table.insert declarations
                    {:catalog :services
                     :id :components.swap
                     :value (fn [db role id]
                              "Return a database with a component role selecting another implementation."
                              (assert (and (= (type role) :string)
                                           (not= role "")
                                           (. (misa.catalog :components) id))
                                      (.. "unknown component: " (tostring id)))
                              (misa.patch db {:components {:roles {role id}}}))})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :app/start
                             :handler (fn [db]
                                        (let [initial (when (not db.components)
                                                        {:roles (collect [role id (pairs configured)]
                                                                  (when (not= role
                                                                              :persist)
                                                                    (assert (and (= (type role)
                                                                                    :string)
                                                                                 (= (type id)
                                                                                    :string))
                                                                            "invalid configured component role")
                                                                    (values role
                                                                            id)))})]
                                          {:patch (when initial
                                                    {:components (misa.replace initial)})
                                           :fx (when (not= config.persist false)
                                                 [{:completion :components/loaded
                                                   :namespace :ui.components
                                                   :type :state/load}])}))}})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :components/loaded
                             :handler (fn [db event]
                                        (if (or (= event.found false)
                                                (= event.data misa.json-null))
                                            nil
                                            (do
                                              (let [saved event.data]
                                                (assert (and (= (type saved)
                                                                :table)
                                                             (= (type saved.roles)
                                                                :table))
                                                        "invalid persisted component selections")
                                                (let [roles {}]
                                                  (each [role id (pairs saved.roles)]
                                                    (assert (and (= (type role)
                                                                    :string)
                                                                 (not= role "")
                                                                 (= (type id)
                                                                    :string))
                                                            "invalid persisted component selection")
                                                    (when (. (misa.catalog :components)
                                                             id)
                                                      (tset roles role id)))
                                                  {:patch {:components {: roles}}})))))}})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :components/swap
                             :handler (fn [db event]
                                        (assert (and (= (type event.role)
                                                        :string)
                                                     (= (type event.implementation)
                                                        :string))
                                                "invalid component swap")
                                        (let [next (misa.components.swap db
                                                                         event.role
                                                                         event.implementation)
                                              fx {}]
                                          (when (not= config.persist false)
                                            (tset fx (+ (length fx) 1)
                                                  {:data next.components
                                                   :namespace :ui.components
                                                   :type :state/save}))
                                          (tset fx (+ (length fx) 1)
                                                {:event {:type :ui/redraw}
                                                 :type :dispatch})
                                          {:patch {:components (misa.replace next.components)}
                                           : fx}))}})
      (definitions.build :components
        declarations
        {:validators {:components (fn [id component]
                                    (assert (and (= (type id) :string)
                                                 (not= id "")
                                                 (= (type component) :table)
                                                 (= (type component.render)
                                                    :function))
                                            "component requires an id and render function"))}}))))

{: build}
