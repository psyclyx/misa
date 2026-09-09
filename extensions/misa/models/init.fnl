;; Model selection policy. Providers own catalogue entries; this extension owns
;; availability and selection. Search and picker transitions belong to picker.

(fn copy-model [model]
  {:api model.api
   :context_window model.context_window
   :id model.id
   :label model.label
   :model model.model
   :created model.created
   :recommended model.recommended
   :popularity_rank model.popularity_rank
   :pricing model.pricing
   :provider model.provider})

(fn find [entries id]
  (accumulate [found nil _ model (ipairs entries) &until found]
    (when (= model.id id) model)))

(fn browse-visible? [model db]
  (let [config (or db.models.catalogue_filter {})
        usage (and db.preferences db.preferences.scopes.models
                   (. db.preferences.scopes.models model.id))]
    (or (= config.enabled false) (= db.models.selected model.id)
        (and usage (or usage.favorite (> (or usage.uses 0) 0)))
        (= model.recommended true)
        (and model.popularity_rank
             (<= model.popularity_rank (or config.popular_limit 30)))
        (and (not= model.recommended false)
             (or (not model.created)
                 (>= model.created
                     (- (or db.models.catalogue_now 0)
                        (* (or config.max_age_days 365) 86400))))))))

(fn rebuild [state preferred]
  (let [entries {}
        preferred (or (and state.roles state.roles.default) preferred)]
    (each [_ model (ipairs state.catalogue)]
      (when (not= (. state.available model.provider) false)
        (tset entries (+ (length entries) 1) model)))
    (let [selected (if (find entries state.selected) state.selected
                       (find entries preferred) preferred
                       (and (= preferred nil) (> (length entries) 0)) (. entries
                                                                         1 :id)
                       nil)]
      (misa.patch state
                  {:entries (misa.replace entries)
                   :selected (misa.replace selected)}))))

(fn updated [state]
  {:patch {:models (misa.replace state)}})

(fn compute-models-indicator [inputs]
  "Project the selected model indicator."
  (let [model (. inputs 1)]
    {:type :text :value (or (and model model.label) :none)}))

(fn selected-model-open [db]
  "Return the selected model for command completion."
  (or (and db.models db.models.selected) nil))

(fn complete-model-open [_ db]
  "Return completion items for available models."
  (let [result {}]
    (each [_ model (ipairs (or (and db.models db.models.entries) {}))]
      (let [api (or model.api {})
            metadata {:type :model
                      :model model.model
                      :provider model.provider
                      :title model.id}]
        (when model.context_window
          (set metadata.context_window model.context_window))
        (when api.request_options
          (set metadata.request_options :available))
        (let [cost (and misa.costs misa.costs.model
                        (misa.costs.model db model.id))]
          (when cost
            (set metadata.cost cost))
          (tset result (+ (length result) 1)
                {:browse_visible (browse-visible? model db)
                 :display {:label (.. model.provider "/" model.model)}
                 :id model.id
                 :path model.id
                 :preview metadata
                 :search [model.id
                          (or model.label "")
                          (or model.model "")
                          (or model.provider "")]
                 :value model.id}))))
    result))

(fn compute-models-selected [inputs]
  "Resolve the selected model from the current catalogue."
  (let [model (find (or (. inputs 1) []) (. inputs 2))]
    (when model
      {:context_window model.context_window
       :id model.id
       :provider model.provider
       :label (.. model.provider "/" model.model)
       :pricing model.pricing})))

(fn compute-models-projection [inputs]
  "Project model selection and its configured fallback."
  {:configured_default (. inputs 1) :selected (. inputs 2)})

(fn on-model-picker-open [db]
  "Open the model selection picker."
  (if (or db.picker db.dialog)
      {:fx [{:type :terminal/read}]}
      {:fx [{:event {:command :/model :type :choices/command-open}
             :type :dispatch}]}))

(fn on-model-open [db event]
  "Select a model from command input."
  (let [state (assert db.models "model state is not initialized")
        requested (or (and (= (type event.arguments) :string)
                           (event.arguments:match "^%s*(%S+)%s*$"))
                      nil)]
    (assert (and requested (find state.entries requested))
            "unknown or unavailable model")
    {:patch {:models {:selected requested}} :fx [{:type :terminal/read}]}))

(fn on-model-select [db event]
  "Select an available model."
  (assert (and (= (type event.id) :string) (find db.models.entries event.id))
          "unknown or unavailable model")
  {:patch {:models {:selected event.id}}})

(fn models-for-role [db role]
  "Resolve the model selected for a named role."
  (let [state db.models]
    (when state
      (find state.entries
            (if (= role :default) state.selected
                (and state.roles (. state.roles role)))))))

(fn complete-model-role [_ db]
  "Return completion items for assigning model roles."
  (let [items []]
    (each [_ role (ipairs [:default :summarizer])]
      (each [_ model (ipairs (or (and db.models db.models.entries) []))]
        (table.insert items {:value (.. role " " model.id)})))
    (table.insert items {:value "summarizer off"})
    items))

(fn on-model-role [db event]
  "Assign and persist a model role."
  (let [(role id) (: (or event.arguments "") :match "^(%S+)%s+(%S+)$")]
    (assert (and role id (or (= role :default) (= role :summarizer))
                 (or (and (= role :summarizer) (= id :off))
                     (find db.models.entries id)))
            "use /role default|summarizer provider/model (or summarizer off)")
    (let [roles (misa.patch (or db.models.roles {})
                            {role (if (= id :off)
                                      misa.delete
                                      id)})]
      {:patch {:models {:roles (misa.replace roles)
                        :selected (when (= role :default)
                                    id)}}
       :fx [{:type :state/save :namespace :model-roles :data roles}
            {:type :terminal/read}]})))

(fn options [config]
  "Validate configured model roles and catalogue filters."
  (let [configured (or config.models {})
        roles (or configured.roles {})
        filter (or configured.catalogue_filter {})
        default (or roles.default configured.default)]
    (assert (= (type roles) :table) "config.models.roles must be an object")
    (each [role id (pairs roles)]
      (assert (and (= (type role) :string) (= (type id) :string) (not= id ""))
              "model roles must map names to nonempty model IDs"))
    (assert (= (type filter) :table)
            "config.models.catalogue_filter must be an object")
    (each [_ key (ipairs [:max_age_days :popular_limit])]
      (let [value (. filter key)]
        (assert (or (= value nil) (and (= (type value) :number) (>= value 0)))
                "model catalogue limits must be nonnegative numbers")))
    (assert (or (= default nil)
                (and (= (type default) :string) (not= default "")))
            "config.models.default must be a nonempty string")
    {: roles :catalogue_filter filter : default}))

(fn on-app-start [config db _ cofx]
  "Initialize the model catalogue and restore saved roles."
  (let [configured (options config)
        default configured.default]
    (when (not db.models)
      (let [result (updated (rebuild {:available (or db.provider_availability
                                                     {})
                                      :catalogue (icollect [_ model (ipairs (misa.models.all))]
                                                   (copy-model model))
                                      :configured_default default
                                      :roles (or (and configured
                                                      configured.roles)
                                                 {})
                                      :catalogue_filter (or (and configured
                                                                 configured.catalogue_filter)
                                                            {})
                                      :catalogue_now (/ (or (and cofx
                                                                 cofx.clock
                                                                 cofx.clock.wall_ms)
                                                            0)
                                                        1000)
                                      :entries {}
                                      :selected default}
                                     default))]
        (set result.fx
             [{:type :state/load
               :namespace :model-roles
               :completion :model/roles-loaded}])
        result))))

(fn on-models-provider-availability [config db event]
  "Update provider availability and reconcile model selection."
  (let [configured (options config)
        default configured.default]
    (assert (and (= (type event.provider) :string)
                 (= (type event.available) :boolean))
            "invalid provider availability")
    (let [state (assert db.models "model state is not initialized")]
      (updated (rebuild (misa.patch state
                                    {:available {event.provider event.available}})
                        default)))))

(fn on-models-update [config db event]
  "Apply metadata updates to a provider model catalogue."
  (let [configured (options config)
        default configured.default]
    (assert (and (= (type event.provider) :string)
                 (= (type event.models) :table))
            "invalid model update")
    (let [state (assert db.models "model state is not initialized")
          updates {}]
      (each [_ update (ipairs event.models)]
        (assert (and (= (type update) :table) (= (type update.id) :string))
                "invalid model update")
        (assert (or (= update.context_window nil)
                    (and (= (type update.context_window) :number)
                         (> update.context_window 0)
                         (= (% update.context_window 1) 0)))
                "invalid context window")
        (tset updates update.id update))
      (let [catalogue {}]
        (each [index model (ipairs state.catalogue)]
          (let [update (or (and (= model.provider event.provider)
                                (. updates model.id))
                           nil)]
            (tset catalogue index (if update
                                      (misa.patch model
                                                  {:context_window (misa.replace update.context_window)
                                                   :api (when (not= update.api
                                                                    nil)
                                                          (misa.replace update.api))
                                                   :pricing (when (not= update.pricing
                                                                        nil)
                                                              (misa.replace update.pricing))})
                                      model))))
        (updated (rebuild (misa.patch state
                                      {:catalogue (misa.replace catalogue)})
                          default))))))

(fn on-models-replace-provider [config db event]
  "Replace discovered provider models and reconcile selection."
  (let [configured (options config)
        default configured.default]
    (assert (and (= (type event.provider) :string) (not= event.provider ""))
            "model provider must be nonempty")
    (assert (= (type event.models) :table) "models must be an array")
    (let [state (assert db.models "model state is not initialized")
          catalogue {}
          seen {}]
      (each [_ model (ipairs state.catalogue)]
        (when (or (not= model.provider event.provider)
                  (and (= model.id state.selected)
                       (not= event.authoritative true)))
          (tset catalogue (+ (length catalogue) 1) model)
          (tset seen model.id true)))
      (each [_ model (ipairs event.models)]
        (assert (and (= (type model) :table) (= (type model.id) :string)
                     (not= model.id ""))
                "invalid discovered model")
        (assert (and (= (type model.model) :string) (not= model.model ""))
                "invalid discovered model ID")
        (assert (or (= model.context_window nil)
                    (and (= (type model.context_window) :number)
                         (> model.context_window 0)
                         (= (% model.context_window 1) 0)))
                "invalid context window")
        (when (. seen model.id)
          (let [index (accumulate [found nil i existing (ipairs catalogue)
                                   &until found]
                        (when (= existing.id model.id) i))]
            (when index (table.remove catalogue index))))
        (tset seen model.id true)
        (tset catalogue (+ (length catalogue) 1)
              {:api model.api
               :context_window model.context_window
               :id model.id
               :created model.created
               :recommended model.recommended
               :popularity_rank model.popularity_rank
               :label (or (and (= (type model.label) :string) model.label)
                          model.id)
               :model model.model
               :pricing model.pricing
               :provider event.provider}))
      (updated (rebuild (misa.patch state {:catalogue (misa.replace catalogue)})
                        default)))))

(fn on-model-roles-loaded [config db event]
  "Restore saved model roles with configured assignments taking precedence."
  (let [configured (options config)
        default configured.default]
    (when (and (not= event.found false) (= (type event.data) :table))
      (let [roles {}]
        (each [role id (pairs event.data)]
          (when (and (= (type role) :string) (= (type id) :string))
            (tset roles role id)))
        (each [role id (pairs (or (and configured configured.roles) {}))]
          (tset roles role id))
        {:patch {:models {:roles (misa.replace roles)
                          :selected (when roles.default
                                      roles.default)}}}))))

{:complete-model-open complete-model-open
 :complete-model-role complete-model-role
 :compute-models-indicator compute-models-indicator
 :compute-models-projection compute-models-projection
 :compute-models-selected compute-models-selected
 :models-for-role models-for-role
 :on-app-start on-app-start
 :on-model-open on-model-open
 :on-model-picker-open on-model-picker-open
 :on-model-role on-model-role
 :on-model-roles-loaded on-model-roles-loaded
 :on-model-select on-model-select
 :on-models-provider-availability on-models-provider-availability
 :on-models-replace-provider on-models-replace-provider
 :on-models-update on-models-update
 :selected-model-open selected-model-open}
