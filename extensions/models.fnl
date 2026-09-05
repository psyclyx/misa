;; Model selection policy. Providers own catalogue entries; this extension owns

;; availability and selection. Search and picker transitions belong to picker.

(fn copy-model [model]
  {:api model.api
   :context_window model.context_window
   :id model.id
   :label model.label
   :model model.model
   :pricing model.pricing
   :provider model.provider})

(fn find [entries id]
  (each [_ model (ipairs entries)] (when (= model.id id) (lua "return model")))
  nil)

(fn select-model [state id] (set state.selected id) nil)

(fn rebuild [state preferred]
  (let [entries {}]
    (each [_ model (ipairs state.catalogue)]
      (when (not= (. state.available model.provider) false)
        (tset entries (+ (length entries) 1) model)))
    (set state.entries entries)
    (when (not (find entries state.selected))
      (set state.selected (or (and (find entries preferred) preferred)
                              (or (and (and (= preferred nil)
                                            (> (length entries) 0))
                                       (. entries 1 :id))
                                  nil))))
    nil))

{:setup (fn [context]
          (when misa.reg_indicator
            (misa.reg_indicator {:icon "◆"
                                 :id :model
                                 :label :model
                                 :value (fn [db]
                                          (local model
                                                 (and misa.selected_model_projection
                                                      (misa.selected_model_projection db)))
                                          (or (and model model.id) :none))}))
          (misa.reg_command {:choice_purpose :models
                             :complete (fn [_ db]
                                         (local result {})
                                         (each [_ model (ipairs (or (and db.models
                                                                         db.models.entries)
                                                                    {}))]
                                           (local api (or model.api {}))
                                           (local metadata
                                                  {:model model.model
                                                   :provider model.provider
                                                   :title model.id})
                                           (when model.context_window
                                             (set metadata.context_window
                                                  model.context_window))
                                           (when api.request_options
                                             (set metadata.request_options
                                                  :available))
                                           (local cost
                                                  (and misa.model_cost_info
                                                       (misa.model_cost_info db
                                                                             model.id)))
                                           (when cost
                                             (set metadata.summary cost.summary)
                                             (set metadata.lines cost.lines)
                                             (when model.context_window
                                               (table.insert metadata.lines 1
                                                             (.. "Context: "
                                                                 model.context_window
                                                                 " tokens"))))
                                           (tset result (+ (length result) 1)
                                                 {:display {:label (.. model.provider
                                                                       "/"
                                                                       model.model)}
                                                  :id model.id
                                                  :path model.id
                                                  :preview metadata
                                                  :search [model.id
                                                           (or model.label "")
                                                           (or model.model "")
                                                           (or model.provider
                                                               "")]
                                                  :value model.id}))
                                         result)
                             :description "Choose the active model"
                             :event :model/open
                             :name :/model
                             :preference_scope :models
                             :selected (fn [db]
                                         (or (and db.models db.models.selected)
                                             nil))})

          (fn misa.selected_model_projection [db]
            (local state (or db.models {}))
            (local model (find (or state.entries {}) state.selected))
            (if (not model) nil
                {:context_window model.context_window
                 :id model.id
                 :label (.. model.provider "/" model.model)
                 :pricing model.pricing}))

          (fn misa.models_projection [db]
            (local selected (misa.selected_model_projection db))
            {:configured_default (. (or db.models {}) :configured_default)
             : selected})

          (local configured (or (and (= (type context.config) :table)
                                     context.config.models)
                                nil))
          (local default (or (and (= (type configured) :table)
                                  configured.default)
                             nil))
          (when (not= default nil)
            (assert (and (= (type default) :string) (not= default ""))
                    "config.models.default must be a nonempty string"))
          (misa.reg_interceptor {:before (fn [tx]
                                           (if (or (not= tx.event.type
                                                         :app/start)
                                                   tx.db.models)
                                               tx
                                               (do
                                                 (local catalogue {})
                                                 (each [_ model (ipairs (misa.models))]
                                                   (tset catalogue
                                                         (+ (length catalogue)
                                                            1)
                                                         (copy-model model)))
                                                 (local state
                                                        {:available (or tx.db.provider_availability
                                                                        {})
                                                         : catalogue
                                                         :configured_default default
                                                         :entries {}
                                                         :selected default})
                                                 (rebuild state default)
                                                 (set tx.db.models state)
                                                 tx)))
                                 :id :models/initialize})
          (misa.reg_event :model/open
                          (fn [db event]
                            (local state
                                   (assert db.models
                                           "model state is not initialized"))
                            (local requested
                                   (or (and (= (type event.arguments) :string)
                                            (event.arguments:match "^%s*(%S+)%s*$"))
                                       nil))
                            (assert (and requested
                                         (find state.entries requested))
                                    "unknown or unavailable model")
                            (select-model state requested)
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :models/provider-availability
                          (fn [db event]
                            (assert (and (= (type event.provider) :string)
                                         (= (type event.available) :boolean))
                                    "invalid provider availability")
                            (local state
                                   (assert db.models
                                           "model state is not initialized"))
                            (tset state.available event.provider
                                  event.available)
                            (rebuild state default)
                            {: db}))
          (misa.reg_event :models/update
                          (fn [db event]
                            (assert (and (= (type event.provider) :string)
                                         (= (type event.models) :table))
                                    "invalid model update")
                            (local state
                                   (assert db.models
                                           "model state is not initialized"))
                            (local updates {})
                            (each [_ update (ipairs event.models)]
                              (assert (and (= (type update) :table)
                                           (= (type update.id) :string))
                                      "invalid model update")
                              (assert (or (= update.context_window nil)
                                          (and (and (= (type update.context_window)
                                                       :number)
                                                    (> update.context_window 0))
                                               (= (% update.context_window 1) 0)))
                                      "invalid context window")
                              (tset updates update.id update))
                            (each [_ model (ipairs state.catalogue)]
                              (local update
                                     (or (and (= model.provider event.provider)
                                              (. updates model.id))
                                         nil))
                              (when update
                                (set model.context_window update.context_window)
                                (when (not= update.api nil)
                                  (set model.api update.api))
                                (when (not= update.pricing nil)
                                  (set model.pricing update.pricing))))
                            (rebuild state default)
                            {: db}))
          (misa.reg_event :models/replace-provider
                          (fn [db event]
                            (assert (and (= (type event.provider) :string)
                                         (not= event.provider ""))
                                    "model provider must be nonempty")
                            (assert (= (type event.models) :table)
                                    "models must be an array")
                            (local (state catalogue seen)
                                   (values (assert db.models
                                                   "model state is not initialized")
                                           {} {}))
                            (each [_ model (ipairs state.catalogue)]
                              (when (or (not= model.provider event.provider)
                                        (and (= model.id state.selected)
                                             (not= event.authoritative true)))
                                (tset catalogue (+ (length catalogue) 1) model)
                                (tset seen model.id true)))
                            (each [_ model (ipairs event.models)]
                              (assert (and (and (= (type model) :table)
                                                (= (type model.id) :string))
                                           (not= model.id ""))
                                      "invalid discovered model")
                              (assert (and (= (type model.model) :string)
                                           (not= model.model ""))
                                      "invalid discovered model ID")
                              (assert (or (= model.context_window nil)
                                          (and (and (= (type model.context_window)
                                                       :number)
                                                    (> model.context_window 0))
                                               (= (% model.context_window 1) 0)))
                                      "invalid context window")
                              (when (. seen model.id)
                                (each [i existing (ipairs catalogue)]
                                  (when (= existing.id model.id)
                                    (table.remove catalogue i)
                                    (lua :break))))
                              (tset seen model.id true)
                              (tset catalogue (+ (length catalogue) 1)
                                    {:api model.api
                                     :context_window model.context_window
                                     :id model.id
                                     :label (or (and (= (type model.label)
                                                        :string)
                                                     model.label)
                                                model.id)
                                     :model model.model
                                     :pricing model.pricing
                                     :provider event.provider}))
                            (set state.catalogue catalogue)
                            (rebuild state default)
                            {: db}))
          (misa.reg_event :model/select
                          (fn [db event]
                            (assert (and (= (type event.id) :string)
                                         (find db.models.entries event.id))
                                    "unknown or unavailable model")
                            (select-model db.models event.id)
                            {: db}))
          nil)}

