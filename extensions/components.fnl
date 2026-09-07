;; Semantic visual component registry. Implementations are immutable registration

;; data; every role selection lives in transactional application state.

(fn shallow [source]
  (local result {})
  (each [key value (pairs source)] (tset result key value))
  result)

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
                                        (each [key value (pairs (misa.theme_style db
                                                                                  (or span.style
                                                                                      :plain)))]
                                          (tset style key value))
                                        (when (or (and span.action (= span.action db.hover_action))
                                                  (and (not span.action) span.link (= span.link db.hover_link)))
                                          (each [key value (pairs (misa.theme_style db
                                                                                    :hover))]
                                            (tset style key value)))
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
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (if (and (= tx.event.type :app/start) (not tx.db.components))
                                             (do
                                               (local roles {})
                                               (each [role id (pairs configured)]
                                                 (when (not= role :persist)
                                                   (assert (and (= (type role)
                                                                   :string)
                                                                (= (type id)
                                                                   :string))
                                                           "invalid configured component role")
                                                   (tset roles role id)))
                                               (misa.patch tx {:db {:components (misa.replace {: roles})}}))
                                             tx))
                                 :id :components/initialize}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (if (= config.persist false) nil
                                        {:fx [{:completion :components/loaded
                                               :namespace :ui.components
                                               :type :state/load}]}))})
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
