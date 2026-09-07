;; Named reactive facts, configured presentation metadata, and one component path.
{:setup (fn [context]
          (local definitions {})
          (local root (or (. (or context.config {}) :status) {}))
          (local configured (or root.indicators
                                [{:id :activity :priority 100 :representation :icon}
                                 {:id :model :priority 90 :representation :value :hotkey true}
                                 {:id :effort :priority 70 :hotkey true}
                                 {:id :session :priority 40 :representation :icon}
                                 {:id :context :priority 80}
                                 {:id :plan :priority 60}
                                 {:id :transcript-detail :priority 20 :hotkey true}]))
          (assert (= (type configured) :table) "config.status.indicators must be an array")
          (local selections
                 (icollect [index item (ipairs configured)]
                   (let [selection (if (= (type item) :string) {:id item} item)
                         representation (or selection.representation :label)]
                     (assert (and (= (type selection) :table) (= (type selection.id) :string))
                             "invalid status indicator selection")
                     (assert (or (= representation :label) (= representation :icon)
                                 (= representation :value) (= representation :label_value))
                             "invalid indicator representation")
                     {:id selection.id : representation :hotkey selection.hotkey
                      :priority (or (tonumber selection.priority) (- 1000 index))})))
          (fn selected []
            (icollect [_ selection (ipairs selections)]
              (when (. definitions selection.id) selection)))
          {:fx [{:type :register/setup-effect :name :register/indicator
                 :handler (fn [effect]
                            (local definition effect.value)
                            (assert (and (= (type definition) :table)
                                         (= (type definition.id) :string) (not= definition.id ""))
                                    "indicator ID must be nonempty")
                            (assert (= definition.value nil) "indicator values must be named queries")
                            (assert (= (type definition.query) :table) "indicator requires a query")
                            (each [_ field (ipairs [:label :icon :action])]
                              (assert (or (= (. definition field) nil) (= (type (. definition field)) :string))
                                      "indicator metadata must be strings"))
                            (assert (or (= definition.hotkey nil)
                                        (and (= (type definition.hotkey) :table)
                                             (= (type definition.hotkey.context) :string)
                                             (= (type definition.hotkey.action) :string)))
                                    "indicator hotkey must name a context and action")
                            (assert (= (. definitions definition.id) nil) "duplicate indicator")
                            (tset definitions definition.id definition)
                            ;; The normal subscription registrar validates query vectors.
                            ;; This boundary validates facts without copying their identities.
                            {:fx [{:type :register/sub
                                   :value {:id (.. :indicators/fact/ definition.id)
                                           :inputs [definition.query]
                                           :compute (fn [inputs]
                                                      (local fact (. inputs 1))
                                                      (assert (or (= fact nil)
                                                                  (and (= (type fact) :table)
                                                                       (= (type fact.type) :string)
                                                                       (not= fact.type "")))
                                                              "indicator query must return a typed fact or nil")
                                                      fact)}}]})}
                {:type :register/sub
                 :value {:id :indicators/model
                         :inputs (fn [] (icollect [_ selection (ipairs (selected))]
                                          [(.. :indicators/fact/ selection.id)]))
                         :compute (fn [inputs]
                                    {:indicators
                                     (icollect [index selection (ipairs (selected))]
                                       (let [fact (. inputs index)
                                             definition (. definitions selection.id)]
                                         (when (not= fact nil)
                                           (local label (if (= selection.representation :icon)
                                                            (or definition.icon definition.label selection.id)
                                                            (or definition.label selection.id)))
                                           (local hotkey
                                                  (if (and (= selection.hotkey true) definition.hotkey
                                                           misa.keybinding_hint)
                                                      (misa.keybinding_hint definition.hotkey.context definition.hotkey.action)
                                                      (= (type selection.hotkey) :string) selection.hotkey nil))
                                           (local action
                                                  (or definition.action
                                                      (when (and definition.hotkey misa.actions)
                                                        (accumulate [found nil _ candidate (ipairs (misa.actions)) &until found]
                                                          (when (and candidate.binding
                                                                     (= candidate.binding.context definition.hotkey.context)
                                                                     (= candidate.binding.action definition.hotkey.action))
                                                            candidate.id)))))
                                           {:id selection.id : label : hotkey : action : fact
                                            :representation selection.representation :priority selection.priority})))})}}
                {:type :register/service :name :indicators_projection
                 :value (fn [db context]
                          (local model (misa.sub db [:indicators/model]))
                          (local presentation (misa.snapshot (or context {})))
                          (when misa.animation_presentation
                            (tset presentation :activity_animation (misa.animation_presentation db :status)))
                          (. (misa.render_component db :status.indicators model presentation) :lines))}]})}
