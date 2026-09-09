(local definitions (require :misa.definitions))

;; Named reactive facts, configured presentation metadata, and one component path.
(fn build [context]
  "Build the declarations for indicators."
  (let [root (or (. (or context.config {}) :status) {})
        configured (or root.indicators
                       [{:id :activity :priority 100 :representation :icon}
                        {:id :model
                         :priority 90
                         :representation :value
                         :hotkey true}
                        {:id :effort :priority 70 :hotkey true}
                        {:id :session :priority 40 :representation :icon}
                        {:id :context :priority 80}
                        {:id :plan :priority 60}
                        {:id :transcript-detail :priority 20 :hotkey true}])]
    (assert (= (type configured) :table)
            "config.status.indicators must be an array")
    (let [overrides (or root.indicator_overrides {})]
      (assert (= (type overrides) :table)
              "config.status.indicator_overrides must be a table")
      (let [selected-ids {}
            entries []]
        (fn add [item]
          (let [selection (if (= (type item) :string) {:id item} item)]
            (assert (and (= (type selection) :table)
                         (= (type selection.id) :string))
                    "invalid indicator selection")
            (assert (not (. selected-ids selection.id))
                    "duplicate indicator selection")
            (tset selected-ids selection.id true)
            (let [override (. overrides selection.id)]
              (when (not= override false)
                (assert (or (= override nil) (= (type override) :table))
                        "indicator override must be a table or false")
                (let [merged (collect [key value (pairs selection)] key value)]
                  (each [key value (pairs (or override {}))]
                    (assert (not= key :id)
                            "indicator override cannot change its id")
                    (tset merged key value))
                  (table.insert entries merged))))))

        (each [_ item (ipairs configured)] (add item))
        (let [additions (icollect [id (pairs overrides)]
                          (do
                            (assert (= (type id) :string)
                                    "indicator override id must be a string")
                            (when (not (. selected-ids id)) id)))]
          (table.sort additions)
          (each [_ id (ipairs additions)] (add {: id}))
          (let [selections (icollect [index item (ipairs entries)]
                             (let [selection (if (= (type item) :string)
                                                 {:id item} item)
                                   representation (or selection.representation
                                                      :label)]
                               (assert (and (= (type selection) :table)
                                            (= (type selection.id) :string))
                                       "invalid status indicator selection")
                               (assert (or (= representation :label)
                                           (= representation :icon)
                                           (= representation :value)
                                           (= representation :label_value))
                                       "invalid indicator representation")
                               {:id selection.id
                                : representation
                                :hotkey selection.hotkey
                                :priority (or (tonumber selection.priority)
                                              (- 1000 index))}))]
            (fn selected []
              (icollect [_ selection (ipairs selections)]
                (when (. (misa.catalog :indicators) selection.id) selection)))

            (definitions.build :indicators
              [(let [definition {:id :indicators/model
                                 :inputs (fn []
                                           (icollect [_ selection (ipairs (selected))]
                                             (. (misa.catalog :indicators)
                                                selection.id :query)))
                                 :compute (fn [inputs]
                                            {:indicators (icollect [index selection (ipairs (selected))]
                                                           (let [fact (. inputs
                                                                         index)
                                                                 definition (. (misa.catalog :indicators)
                                                                               selection.id)]
                                                             (when (not= fact
                                                                         nil)
                                                               (assert (and (= (type fact)
                                                                               :table)
                                                                            (= (type fact.type)
                                                                               :string)
                                                                            (not= fact.type
                                                                                  ""))
                                                                       "indicator query must return a typed fact or nil")
                                                               (let [label (if (= selection.representation
                                                                                  :icon)
                                                                               (or definition.icon
                                                                                   definition.label
                                                                                   selection.id)
                                                                               (or definition.label
                                                                                   selection.id))
                                                                     hotkey (if (and (= selection.hotkey
                                                                                        true)
                                                                                     definition.hotkey
                                                                                     misa.keybindings
                                                                                     misa.keybindings.hint)
                                                                                (misa.keybindings.hint definition.hotkey.context
                                                                                                       definition.hotkey.action)
                                                                                (= (type selection.hotkey)
                                                                                   :string)
                                                                                selection.hotkey
                                                                                nil)
                                                                     action (or definition.action
                                                                                (when (and definition.hotkey
                                                                                           misa.actions
                                                                                           misa.actions.all)
                                                                                  (accumulate [found nil _ candidate (ipairs (misa.actions.all))
                                                                                               &until found]
                                                                                    (when (and candidate.binding
                                                                                               (= candidate.binding.context
                                                                                                  definition.hotkey.context)
                                                                                               (= candidate.binding.action
                                                                                                  definition.hotkey.action))
                                                                                      candidate.id))))]
                                                                 {:id selection.id
                                                                  : label
                                                                  : hotkey
                                                                  : action
                                                                  : fact
                                                                  :representation selection.representation
                                                                  :priority selection.priority}))))})}]
                 {:catalog :subscriptions
                  :id (. definition :id)
                  :value definition})
               {:catalog :services
                :id :status.indicators
                :value (fn [db context]
                         "Render the configured status indicators."
                         (let [model (misa.sub db [:indicators/model])
                               presentation (misa.snapshot (or context {}))]
                           (when (and misa.animations misa.animations.state)
                             (tset presentation :activity_animation
                                   (misa.animations.state db :status)))
                           (. (misa.components.render db :status.indicators
                                                      model presentation)
                              :lines)))}]
              {:validators {:indicators (fn [id definition]
                                          (assert (and (= (type definition)
                                                          :table)
                                                       (= definition.id id)
                                                       (= definition.value nil)
                                                       (= (type definition.query)
                                                          :table)
                                                       (= (type (. definition.query
                                                                   1))
                                                          :string)
                                                       (not= (. definition.query
                                                                1)
                                                             ""))
                                                  "indicator requires an id and named query")
                                          (each [_ field (ipairs [:label
                                                                  :icon
                                                                  :action])]
                                            (assert (or (= (. definition field)
                                                           nil)
                                                        (= (type (. definition
                                                                    field))
                                                           :string))
                                                    "indicator metadata must be strings"))
                                          (assert (or (= definition.hotkey nil)
                                                      (and (= (type definition.hotkey)
                                                              :table)
                                                           (= (type definition.hotkey.context)
                                                              :string)
                                                           (= (type definition.hotkey.action)
                                                              :string)))
                                                  "indicator hotkey must name context and action"))}})))))))

{: build}
