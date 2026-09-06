;; Semantic visual component registry. Implementations are immutable registration

;; data; every role selection lives in transactional application state.

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
                         :name :render_component
                         :value (fn [db role model render-context]
                                  (local component (misa.component db role))
                                  (local rendered
                                         (component.render (misa.snapshot model)
                                                           (misa.snapshot (or render-context
                                                                              {}))))
                                  (assert (= (type rendered) :table)
                                          "component render must return a table")
                                  ;; Components emit semantic tokens (or ordered token lists) and know
                                  ;; nothing about terminal colors. Resolution composes one native record.
                                  (assert misa.theme_style
                                          "components requires the themes service")
                                  (each [_ line (ipairs (or rendered.lines {}))]
                                    (local surface
                                           (or line.surface rendered.surface))
                                    (local base
                                           (or (and surface
                                                    (misa.theme_style db
                                                                      surface))
                                               {}))
                                    (var text "")
                                    (each [_ span (ipairs (or line.spans {}))]
                                      (local style {})
                                      (each [key value (pairs base)]
                                        (tset style key value))
                                      (each [key value (pairs (misa.theme_style db
                                                                                (or span.style
                                                                                    :plain)))]
                                        (tset style key value))
                                      (set span.style style)
                                      (when span.animation
                                        (each [_ frame (ipairs (or span.animation.frames
                                                                   {}))]
                                          (when frame.style
                                            (local resolved {})
                                            (each [key value (pairs style)]
                                              (tset resolved key value))
                                            (each [key value (pairs (misa.theme_style db
                                                                                      frame.style))]
                                              (tset resolved key value))
                                            (set frame.style resolved))))
                                      (set text (.. text (or span.text ""))))
                                    ;; A surface belongs to the box, including the unused part of each row.
                                    (when (and (and (and surface render-context)
                                                    render-context.columns)
                                               misa.layout)
                                      (local padding
                                             (math.max 0
                                                       (- render-context.columns
                                                          (misa.layout.width text))))
                                      (when (> padding 0)
                                        (tset line.spans
                                              (+ (length line.spans) 1)
                                              {:style base
                                               :text (string.rep " " padding)}))))
                                  rendered)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :swap_component
                         :value (fn [db role id]
                                  (assert (and (and (= (type role) :string)
                                                    (not= role ""))
                                               (. implementations id))
                                          (.. "unknown component: "
                                              (tostring id)))
                                  (tset db.components.roles role id)
                                  nil)})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (when (= tx.event.type :app/start)
                                             (when (not tx.db.components)
                                               (local roles {})
                                               (each [role id (pairs configured)]
                                                 (when (not= role :persist)
                                                   (assert (and (= (type role)
                                                                   :string)
                                                                (= (type id)
                                                                   :string))
                                                           "invalid configured component role")
                                                   (tset roles role id)))
                                               (set tx.db.components {: roles})))
                                           tx)
                                 :id :components/initialize}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (if (= config.persist false) {: db}
                                        {: db
                                         :fx [{:completion :components/loaded
                                               :namespace :ui.components
                                               :type :state/load}]}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :components/loaded
                         :handler (fn [db event]
                                    (if (or (= event.found false)
                                            (= event.data misa.json_null))
                                        {: db}
                                        (do
                                          (local saved event.data)
                                          (assert (and (= (type saved) :table)
                                                       (= (type saved.roles)
                                                          :table))
                                                  "invalid persisted component selections")
                                          (each [role id (pairs saved.roles)]
                                            (assert (and (and (= (type role)
                                                                 :string)
                                                              (not= role ""))
                                                         (= (type id) :string))
                                                    "invalid persisted component selection")
                                            (when (. implementations id)
                                              (tset db.components.roles role id)))
                                          {: db})))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :components/swap
                         :handler (fn [db event]
                                    (assert (and (= (type event.role) :string)
                                                 (= (type event.implementation)
                                                    :string))
                                            "invalid component swap")
                                    (misa.swap_component db event.role
                                                         event.implementation)
                                    (local fx {})
                                    (when (not= config.persist false)
                                      (tset fx (+ (length fx) 1)
                                            {:data db.components
                                             :namespace :ui.components
                                             :type :state/save}))
                                    (tset fx (+ (length fx) 1)
                                          {:event {:type :ui/redraw}
                                           :type :dispatch})
                                    {: db : fx})})
          {:fx setup-fx})}
