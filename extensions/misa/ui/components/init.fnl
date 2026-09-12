(local {:view validate : integer?} (require :misa.ui.components.validation))

;; Semantic visual component registry. Implementations are immutable registration
;; data; every role selection lives in transactional application state.

(fn shallow [source]
  (let [result {}]
    (each [key value (pairs source)] (tset result key value))
    result))

(fn same-fields? [a b]
  (or (= a b) (and a b (accumulate [same? true key value (pairs a)
                                    &until (not same?)]
                         (= value (. b key)))
                   (accumulate [same? true key value (pairs b)
                                &until (not same?)]
                     (= value (. a key))))))

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

(fn failure [role context diagnostic]
  (let [message (.. "Component " role " failed")
        requested (and (= (type context) :table) context.columns)
        columns (if (integer? requested 0 65535) requested (length message))
        text (if (and misa.layout misa.layout.clip)
                 (misa.layout.clip message (math.max 0 columns))
                 (: (message:gsub "[\128-\255]" "?") :sub 1
                    (math.max 0 columns)))]
    {:component_error {: role :detail (tostring diagnostic)}
     :lines [{:content false
              :component_error {: role :detail (tostring diagnostic)}
              :spans [{: text}]}]}))

(fn components-lookup [db role]
  "Resolve the implementation selected for a component role."
  (assert (= (type db) :table) "component resolution requires db")
  (assert (and (= (type role) :string) (not= role ""))
          "component role must be nonempty")
  (let [state (assert db.components "component state is not initialized")
        id (or (. state.roles role) (.. :default. role))]
    (assert (and (= (type id) :string) (not= id ""))
            (.. "no component configured for role: " role))
    (assert (. (misa.catalog :components) id)
            (.. "unknown component for " role ": " id))))

(fn components-resolve [db rendered render-context]
  "Resolve semantic component styles without mutating the source."
  (assert (= (type rendered) :table) "component render must return a table")
  ;; Components emit semantic tokens (or ordered token lists) and know
  ;; nothing about terminal colors. Resolution composes one native record.
  (assert misa.themes.style "components requires the themes service")
  ;; Resolve into output records; component-owned semantic caches remain reusable.
  (let [result (shallow rendered)]
    (when rendered.lines
      (set result.lines [])
      (each [_ line (ipairs rendered.lines)]
        (let [resolved-line (shallow line)]
          (set resolved-line.spans [])
          (table.insert result.lines resolved-line)
          (let [surface (or line.surface rendered.surface)
                base (if surface
                         (misa.themes.style db surface)
                         {})
                texts []]
            (each [_ span (ipairs (or line.spans []))]
              (let [resolved-span (shallow span)
                    style (shallow base)
                    disabled (and span.action misa.choices misa.choices.pending
                                  (misa.choices.pending db))]
                (each [key value (pairs (misa.themes.style db
                                                           (or span.style
                                                               :plain)))]
                  (tset style key value))
                (when (and (not disabled)
                           (or (and span.action (= span.action db.hover_action))
                               (and (not span.action) span.link
                                    (= span.link db.hover_link))))
                  (each [key value (pairs (misa.themes.style db :hover))]
                    (tset style key value)))
                (when disabled
                  (when (not span.sequence_progress)
                    (each [key value (pairs (misa.themes.style db :disabled))]
                      (tset style key value)))
                  (set resolved-span.action nil))
                (set resolved-span.style style)
                (when span.animation
                  (var animation nil)
                  (each [index frame (ipairs (or span.animation.frames []))]
                    (when frame.style
                      (when (not animation)
                        (set animation (shallow span.animation))
                        (set animation.frames (shallow span.animation.frames))
                        (set resolved-span.animation animation))
                      (let [resolved-frame (shallow frame)
                            frame-style (shallow style)]
                        (each [key value (pairs (misa.themes.style db
                                                                   frame.style))]
                          (tset frame-style key value))
                        (when (and disabled (not span.sequence_progress))
                          (each [key value (pairs (misa.themes.style db
                                                                     :disabled))]
                            (tset frame-style key value)))
                        (set resolved-frame.style frame-style)
                        (tset animation.frames index resolved-frame)))))
                (table.insert resolved-line.spans resolved-span)
                (table.insert texts (or span.text ""))))
            ;; A surface belongs to the box, including unused row cells.
            (when (and surface render-context render-context.columns
                       misa.layout)
              (let [padding (math.max 0
                                      (- render-context.columns
                                         (misa.layout.width (table.concat texts))))]
                (when (> padding 0)
                  (table.insert resolved-line.spans
                                {:style base :text (string.rep " " padding)}))))))))
    result))

(fn invoke [render-child db role model context cache]
  (let [component (misa.components.lookup db role)
        context (if component.compose
                    (shallow (or context {}))
                    (or context {}))]
    (when component.compose
      (set context.render_child
           (fn [role model child]
             (render-child db role model (or child context)))))
    (let [(view next-cache) (component.render model context cache)]
      (values (validate view) next-cache))))

(fn render-safe [db role model context cache]
  (let [(ok rendered next-cache) (pcall invoke render-safe db role model
                                        context cache)]
    (if ok (values rendered next-cache)
        (values (failure role context rendered) nil))))

(fn resolve-safe [db role source context]
  (let [(ok view) (pcall misa.components.resolve db source context)]
    (if ok view (failure role context view))))

(fn components-render [db role model render-context]
  "Render a selected component and contain presentation failures."
  (let [rendered (render-safe db role model render-context)]
    (resolve-safe db role rendered render-context)))

(fn components-entry [db role model render-context previous theme]
  "Resolve one collection item, reusing the previous entry when it still applies.

The collection below is a map over this step. It is also the unit a consumer with
its own indexing needs: only the entries whose inputs changed are rebuilt, so a
windowed consumer can retain per-item geometry without restating these rules."
  (let [old previous
        (found selected) (pcall misa.components.lookup db role)
        component (and found selected)
        same (and old
                  (or (and component (not component.compose))
                      (= db.components old.components))
                  (= component old.component) (same-fields? model old.model)
                  (same-fields? render-context old.context))
        (source cache) (if same
                           (values old.source old.cache)
                           (render-safe db role model render-context
                                        (and old (= component old.component)
                                             old.cache)))
        (actions links) (if same (values old.actions old.links)
                            (hover-targets source))
        hover-action (and db.hover_action (. actions db.hover_action)
                          db.hover_action)
        hover-link (and db.hover_link (. links db.hover_link) db.hover_link)
        choice-pending (and misa.choices misa.choices.pending
                            (misa.choices.pending db))]
    (if (and same (= theme old.theme) (= hover-action old.hover_action)
             (= hover-link old.hover_link) (= choice-pending old.choice_pending))
        old
        {: component
         :components db.components
         : model
         :context render-context
         : source
         : cache
         : actions
         : links
         : theme
         :hover_action hover-action
         :hover_link hover-link
         :choice_pending choice-pending
         :view (resolve-safe db role source render-context)})))

(fn components-projection-value [inputs _ previous]
  "Project a component collection while retaining compatible cache entries."
  (let [db (. inputs 1)
        items (. inputs 2)
        render-context (. inputs 3)
        theme (misa.themes.lookup db)
        entries {}
        views []]
    (each [_ item (ipairs items)]
      (assert (and (= (type item.id) :string) (not= item.id ""))
              "component collection requires unique nonempty ids")
      (assert (not (. entries item.id))
              "component collection requires unique nonempty ids")
      (let [entry (components-entry db item.role item.model render-context
                                    (and previous (. previous.entries item.id))
                                    theme)]
        (tset entries item.id entry)
        (table.insert views entry.view)))
    {: entries : views}))

(fn components-project [db id items render-context]
  "Project an ordered component collection with reusable per-item caches."
  (assert (and (= (type id) :string) (not= id ""))
          "component collection requires an owner id")
  (misa.sub {: db : items :context (or render-context {})}
            [:components/projection id]))

(fn components-swap [db role id]
  "Return a database with a component role selecting another implementation."
  (assert (and (= (type role) :string) (not= role "")
               (. (misa.catalog :components) id))
          (.. "unknown component: " (tostring id)))
  (misa.patch db {:components {:roles {role id}}}))

(fn app-start [config configured db]
  "Initialize selected presentation state and request persisted choices."
  (let [initial (when (not db.components)
                  {:roles (collect [role id (pairs configured)]
                            (when (not= role :persist)
                              (assert (and (= (type role) :string)
                                           (= (type id) :string))
                                      "invalid configured component role")
                              (values role id)))})]
    {:patch (when initial
              {:components (misa.replace initial)})
     :fx (when (not= config.persist false)
           [{:completion :components/loaded
             :namespace :ui.components
             :type :state/load}])}))

(fn components-loaded [db event]
  "Restore available component role selections."
  (if (or (= event.found false) (= event.data misa.json-null))
      nil
      (let [saved event.data]
        (assert (and (= (type saved) :table) (= (type saved.roles) :table))
                "invalid persisted component selections")
        (let [roles {}]
          (each [role id (pairs saved.roles)]
            (assert (and (= (type role) :string) (not= role "")
                         (= (type id) :string))
                    "invalid persisted component selection")
            (when (. (misa.catalog :components) id)
              (tset roles role id)))
          {:patch {:components {: roles}}}))))

(fn components-swap-handler [config db event]
  "Select a component role and plan persistence and redraw effects."
  (assert (and (= (type event.role) :string)
               (= (type event.implementation) :string))
          "invalid component swap")
  (let [next (misa.components.swap db event.role event.implementation)
        fx {}]
    (when (not= config.persist false)
      (tset fx (+ (length fx) 1)
            {:data next.components :namespace :ui.components :type :state/save}))
    (tset fx (+ (length fx) 1) {:event {:type :ui/redraw} :type :dispatch})
    {:patch {:components (misa.replace next.components)} : fx}))

(fn validate-component [id component]
  "Require a component with a callable renderer."
  (assert (and (= (type id) :string) (not= id "") (= (type component) :table)
               (= (type component.render) :function))
          "component requires an id and render function"))

{: app-start
 : components-entry
 : components-loaded
 : components-lookup
 : components-project
 : components-projection-value
 : components-render
 : components-resolve
 : components-swap
 : components-swap-handler
 : validate-component}
