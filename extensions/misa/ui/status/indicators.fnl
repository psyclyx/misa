;; Named reactive facts, configured presentation metadata, and one component path.
(fn indicators-model-value [selected inputs]
  "Combine selected indicator metadata with reactive facts."
  {:indicators (icollect [index selection (ipairs (selected))]
                 (let [fact (. inputs index)
                       definition (. (misa.catalog :indicators) selection.id)]
                   (when (not= fact nil)
                     (assert (and (= (type fact) :table)
                                  (= (type fact.type) :string)
                                  (not= fact.type ""))
                             "indicator query must return a typed fact or nil")
                     (let [label (if (= selection.representation :icon)
                                     (or definition.icon definition.label
                                         selection.id)
                                     (or definition.label selection.id))
                           hotkey (if (and (= selection.hotkey true)
                                           definition.hotkey misa.keybindings
                                           misa.keybindings.hint)
                                      (misa.keybindings.hint definition.hotkey.context
                                                             definition.hotkey.action)
                                      (= (type selection.hotkey) :string)
                                      selection.hotkey
                                      nil)
                           action (or definition.action
                                      (when (and definition.hotkey misa.actions
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
                        :priority selection.priority}))))})

(fn status-indicators [db context]
  "Render the configured status indicators."
  (let [model (misa.sub db [:indicators/model])
        presentation (misa.snapshot (or context {}))]
    (when (and misa.animations misa.animations.state)
      (tset presentation :activity_animation (misa.animations.state db :status)))
    (. (misa.components.render db :status.indicators model presentation) :lines)))

(fn validate-indicator [id definition]
  "Validate the query and presentation metadata of an indicator."
  (assert (and (= (type definition) :table) (= definition.id id)
               (= definition.value nil) (= (type definition.query) :table)
               (= (type (. definition.query 1)) :string)
               (not= (. definition.query 1) ""))
          "indicator requires an id and named query")
  (each [_ field (ipairs [:label :icon :action])]
    (assert (or (= (. definition field) nil)
                (= (type (. definition field)) :string))
            "indicator metadata must be strings"))
  (assert (or (= definition.hotkey nil)
              (and (= (type definition.hotkey) :table)
                   (= (type definition.hotkey.context) :string)
                   (= (type definition.hotkey.action) :string)))
          "indicator hotkey must name context and action"))

(fn selections [configured]
  "Validate an ordered indicator selection without changing its values."
  (assert (= (type configured) :table) "status indicators must be an array")
  (let [seen {}]
    (icollect [index value (ipairs configured)]
      (let [selection (if (= (type value) :string) {:id value} value)
            representation (or selection.representation :label)]
        (assert (and (= (type selection) :table)
                     (= (type selection.id) :string))
                "invalid indicator selection")
        (assert (not (. seen selection.id)) "duplicate indicator selection")
        (tset seen selection.id true)
        (assert (or (= representation :label) (= representation :icon)
                    (= representation :value) (= representation :label_value))
                "invalid indicator representation")
        {:id selection.id
         :representation representation
         :hotkey selection.hotkey
         :priority (or (tonumber selection.priority) (- 1000 index))}))))

{: indicators-model-value
 : selections
 : status-indicators
 : validate-indicator}
