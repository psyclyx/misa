;; Semantic visual component registry. Implementations are immutable registration

;; data; every role selection lives in transactional application state.

(fn shallow [source]
  (local result {})
  (each [key value (pairs source)] (tset result key value))
  result)

(fn same-fields [a b]
  (if (= a b) true
      (or (not a) (not b)) false
      (do
        (each [key value (pairs a)] (when (not= value (. b key)) (lua "return false")))
        (each [key value (pairs b)] (when (not= value (. a key)) (lua "return false")))
        true)))

(fn hover-targets [rendered]
  (local actions {})
  (local links {})
  (each [_ line (ipairs (or rendered.lines []))]
    (each [_ span (ipairs (or line.spans []))]
      (if span.action (tset actions span.action true)
          span.link (tset links span.link true))))
  (values actions links))

{:setup (fn [context]
          (local setup-fx [])
          (local implementations {})
          (var config (or (and (= (type context.config) :table)
                               context.config.components)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local configured (or (and (= (type config.roles) :table)
                                     config.roles)
                                config))
          (table.insert setup-fx
                        {:type :register/setup-effect
                         :name :register/component
                         :handler (fn [effect]
                                    (let [id effect.id
                                          component effect.value]
                                      (assert (and (= (type id) :string)
                                                   (not= id ""))
                                              "component ID must be nonempty")
                                      (assert (and (= (type component) :table)
                                                   (= (type component.render)
                                                      :function))
                                              "component must provide render")
                                      (assert (= (. implementations id) nil)
                                              (.. "duplicate component: " id))
                                      (tset implementations id component)
                                      nil))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :component
                         :value (fn [db role]
                                  (assert (= (type db) :table)
                                          "component resolution requires db")
                                  (assert (and (= (type role) :string)
                                               (not= role ""))
                                          "component role must be nonempty")
                                  (local state
                                         (assert db.components
                                                 "component state is not initialized"))
                                  (local id
                                         (or (. state.roles role)
                                             (.. :default. role)))
                                  (assert (and (= (type id) :string)
                                               (not= id ""))
                                          (.. "no component configured for role: "
                                              role))
                                  (assert (. implementations id)
                                          (.. "unknown component for " role
                                              ": " id)))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :resolve_component
                         :value (fn [db rendered render-context]
                                  (assert (= (type rendered) :table)
                                          "component render must return a table")
                                  ;; Components emit semantic tokens (or ordered token lists) and know
                                  ;; nothing about terminal colors. Resolution composes one native record.
                                  (assert misa.theme_style
                                          "components requires the themes service")
                                  ;; Resolve into output records; component-owned semantic caches remain reusable.
                                  (local result (shallow rendered))
                                  (when rendered.lines
                                    (set result.lines [])
                                    (each [_ line (ipairs rendered.lines)]
                                      (local resolved-line (shallow line))
                                      (set resolved-line.spans [])
                                      (table.insert result.lines resolved-line)
                                      (local surface
                                             (or line.surface rendered.surface))
                                      (local base
                                             (if surface
                                                 (misa.theme_style db surface)
                                                 {}))
                                      (local texts [])
                                      (each [_ span (ipairs (or line.spans []))]
                                        (local resolved-span (shallow span))
                                        (local style (shallow base))
                                        (local disabled (and span.action misa.choice_pending (misa.choice_pending db)))
                                        (each [key value (pairs (misa.theme_style db
                                                                                  (or span.style
                                                                                      :plain)))]
                                          (tset style key value))
                                        (when (and (not disabled)
                                                   (or (and span.action (= span.action db.hover_action))
                                                       (and (not span.action) span.link (= span.link db.hover_link))))
                                          (each [key value (pairs (misa.theme_style db
                                                                                    :hover))]
                                            (tset style key value)))
                                        (when disabled
                                          (when (not span.sequence_progress)
                                            (each [key value (pairs (misa.theme_style db :disabled))]
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
                                              (local resolved-frame
                                                     (shallow frame))
                                              (local frame-style
                                                     (shallow style))
                                              (each [key value (pairs (misa.theme_style db
                                                                                        frame.style))]
                                                (tset frame-style key value))
                                              (when (and disabled (not span.sequence_progress))
                                                (each [key value (pairs (misa.theme_style db :disabled))]
                                                  (tset frame-style key value)))
                                              (set resolved-frame.style
                                                   frame-style)
                                              (tset animation.frames index
                                                    resolved-frame))))
                                        (table.insert resolved-line.spans
                                                      resolved-span)
                                        (table.insert texts (or span.text "")))
                                      ;; A surface belongs to the box, including unused row cells.
                                      (when (and surface render-context
                                                 render-context.columns
                                                 misa.layout)
                                        (local padding
                                               (math.max 0
                                                         (- render-context.columns
                                                            (misa.layout.width (table.concat texts)))))
                                        (when (> padding 0)
                                          (table.insert resolved-line.spans
                                                        {:style base
                                                         :text (string.rep " "
                                                                           padding)})))))
                                  result)})
          ;; Child views share role selections and retain semantic styles.
          (fn child-context [db context]
            (local next (shallow (or context {})))
            (set next.render_child
                 (fn [role model child]
                   (local component (misa.component db role))
                   (local context (or child next))
                   (component.render model (if component.compose (child-context db context) context))))
            next)
          (table.insert setup-fx
                        {:type :register/service :name :render_component
                         :value (fn [db role model render-context]
                                  (local component (misa.component db role))
                                  (local rendered (component.render model
                                                    (if component.compose (child-context db render-context)
                                                        (or render-context {}))))
                                  (misa.resolve_component db rendered render-context))})
          (table.insert setup-fx
                        {:type :register/sub
                         :value {:id :components/projection
                                 :inputs [[:db/path :db] [:db/path :items] [:db/path :context]]
                                 :compute (fn [inputs _ previous]
                                            (local db (. inputs 1))
                                            (local items (. inputs 2))
                                            (local render-context (. inputs 3))
                                            (local theme (misa.theme db))
                                            (local entries {})
                                            (local views [])
                                            (each [_ item (ipairs items)]
                                              (assert (and (= (type item.id) :string) (not= item.id "")
                                                           (not (. entries item.id)))
                                                      "component collection requires unique nonempty ids")
                                              (local old (and previous (. previous.entries item.id)))
                                              (local component (misa.component db item.role))
                                              (local same (and old (or (not component.compose) (= db.components old.components)) (= component old.component)
                                                               (same-fields item.model old.model)
                                                               (same-fields render-context old.context)))
                                              (local (source cache)
                                                     (if same (values old.source old.cache)
                                                         (component.render item.model (if component.compose (child-context db render-context) render-context)
                                                                           (and old (= component old.component) old.cache))))
                                              (local (actions links) (if same (values old.actions old.links)
                                                                        (hover-targets source)))
                                              (local hover-action (and db.hover_action (. actions db.hover_action) db.hover_action))
                                              (local hover-link (and db.hover_link (. links db.hover_link) db.hover_link))
                                              (local entry
                                                     (if (and same (= theme old.theme)
                                                              (= hover-action old.hover_action) (= hover-link old.hover_link)
                                                              (= (and misa.choice_pending (misa.choice_pending db)) old.choice_pending))
                                                         old
                                                         {: component :components db.components :model item.model :context render-context
                                                          : source : cache : actions : links : theme
                                                          :hover_action hover-action :hover_link hover-link
                                                          :choice_pending (and misa.choice_pending (misa.choice_pending db))
                                                          :view (misa.resolve_component db source render-context)}))
                                              (tset entries item.id entry)
                                              (table.insert views entry.view))
                                            {: entries : views})}})
          (table.insert setup-fx
                        {:type :register/service :name :project_components
                         :value (fn [db id items render-context]
                                  (assert (and (= (type id) :string) (not= id ""))
                                          "component collection requires an owner id")
                                  (misa.sub {: db : items :context (or render-context {})}
                                            [:components/projection id]))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :swap_component
                         :value (fn [db role id]
                                  (assert (and (and (= (type role) :string)
                                                    (not= role ""))
                                               (. implementations id))
                                          (.. "unknown component: "
                                              (tostring id)))
                                  (misa.patch db {:components {:roles {role id}}}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (local initial
                                           (when (not db.components)
                                             {:roles (collect [role id (pairs configured)]
                                                       (when (not= role :persist)
                                                         (assert (and (= (type role) :string)
                                                                      (= (type id) :string))
                                                                 "invalid configured component role")
                                                         (values role id)))}))
                                    {:patch (when initial {:components (misa.replace initial)})
                                     :fx (when (not= config.persist false)
                                          [{:completion :components/loaded
                                               :namespace :ui.components
                                               :type :state/load}])})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :components/loaded
                         :handler (fn [db event]
                                    (if (or (= event.found false)
                                            (= event.data misa.json_null))
                                        nil
                                        (do
                                          (local saved event.data)
                                          (assert (and (= (type saved) :table)
                                                       (= (type saved.roles)
                                                          :table))
                                                  "invalid persisted component selections")
                                          (local roles {})
                                          (each [role id (pairs saved.roles)]
                                            (assert (and (and (= (type role)
                                                                 :string)
                                                              (not= role ""))
                                                         (= (type id) :string))
                                                    "invalid persisted component selection")
                                            (when (. implementations id)
                                              (tset roles role id)))
                                          {:patch {:components {: roles}}})))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :components/swap
                         :handler (fn [db event]
                                    (assert (and (= (type event.role) :string)
                                                 (= (type event.implementation)
                                                    :string))
                                            "invalid component swap")
                                    (local next (misa.swap_component db event.role
                                                                    event.implementation))
                                    (local fx {})
                                    (when (not= config.persist false)
                                      (tset fx (+ (length fx) 1)
                                            {:data next.components
                                             :namespace :ui.components
                                             :type :state/save}))
                                    (tset fx (+ (length fx) 1)
                                          {:event {:type :ui/redraw}
                                           :type :dispatch})
                                    {:patch {:components (misa.replace next.components)} : fx})})
          {:fx setup-fx})}
