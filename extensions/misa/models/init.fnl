(local definitions (require :misa.definitions))

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
  (each [_ model (ipairs entries)] (when (= model.id id) (lua "return model")))
  nil)

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

(fn build [context]
  "Build the declarations for models."
  (let [declarations []]
    (do
      (table.insert declarations
                    (let [definition {:icon "◆"
                                      :id :model
                                      :label :model
                                      :hotkey {:action :open_model_picker
                                               :context :global}
                                      :query [:models/indicator]}]
                      {:catalog :indicators
                       :id (. definition :id)
                       :value definition})))
    (table.insert declarations
                  (let [definition {:id :models/indicator
                                    :inputs [[:models/selected]]
                                    :compute (fn [inputs]
                                               (let [model (. inputs 1)]
                                                 {:type :text
                                                  :value (or (and model
                                                                  model.label)
                                                             :none)}))}]
                    {:catalog :subscriptions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:action :open_model_picker
                                    :context :global
                                    :default [:alt+m]}]
                    {:catalog :keybindings
                     :id (.. (. definition :context) "/" (. definition :action))
                     :value definition}))
    (table.insert declarations
                  (let [definition {:binding {:action :open_model_picker
                                              :context :global}
                                    :event {:type :model/picker-open}
                                    :id :models.open
                                    :label "Choose active model"}]
                    {:catalog :actions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:choice_purpose :models
                                    :complete (fn [_ db]
                                                (let [result {}]
                                                  (each [_ model (ipairs (or (and db.models
                                                                                  db.models.entries)
                                                                             {}))]
                                                    (let [api (or model.api {})
                                                          metadata {:type :model
                                                                    :model model.model
                                                                    :provider model.provider
                                                                    :title model.id}]
                                                      (when model.context_window
                                                        (set metadata.context_window
                                                             model.context_window))
                                                      (when api.request_options
                                                        (set metadata.request_options
                                                             :available))
                                                      (let [cost (and misa.costs
                                                                      misa.costs.model
                                                                      (misa.costs.model db
                                                                                        model.id))]
                                                        (when cost
                                                          (set metadata.cost
                                                               cost))
                                                        (tset result
                                                              (+ (length result)
                                                                 1)
                                                              {:browse_visible (browse-visible? model
                                                                                                db)
                                                               :display {:label (.. model.provider
                                                                                    "/"
                                                                                    model.model)}
                                                               :id model.id
                                                               :path model.id
                                                               :preview metadata
                                                               :search [model.id
                                                                        (or model.label
                                                                            "")
                                                                        (or model.model
                                                                            "")
                                                                        (or model.provider
                                                                            "")]
                                                               :value model.id}))))
                                                  result))
                                    :description "Choose the active model"
                                    :event :model/open
                                    :name :/model
                                    :preference_scope :models
                                    :selected (fn [db]
                                                (or (and db.models
                                                         db.models.selected)
                                                    nil))}]
                    {:catalog :commands
                     :id (. definition :name)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:id :models/selected
                                    :inputs [[:db/path :models :entries]
                                             [:db/path :models :selected]]
                                    :compute (fn [inputs]
                                               (let [model (find (or (. inputs
                                                                        1)
                                                                     [])
                                                                 (. inputs 2))]
                                                 (when model
                                                   {:context_window model.context_window
                                                    :id model.id
                                                    :provider model.provider
                                                    :label (.. model.provider
                                                               "/" model.model)
                                                    :pricing model.pricing})))}]
                    {:catalog :subscriptions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:id :models/projection
                                    :inputs [[:db/path
                                              :models
                                              :configured_default]
                                             [:models/selected]]
                                    :compute (fn [inputs]
                                               {:configured_default (. inputs 1)
                                                :selected (. inputs 2)})}]
                    {:catalog :subscriptions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  {:catalog :services
                   :id :models.selected
                   :value (fn [db]
                            "Return the currently selected model."
                            (misa.sub db [:models/selected]))})
    (table.insert declarations
                  {:catalog :services
                   :id :models.state
                   :value (fn [db]
                            "Return the current model catalogue and selection state."
                            (misa.sub db [:models/projection]))})
    (let [configured (or (and (= (type context.config) :table)
                              context.config.models)
                         nil)]
      (when (and configured configured.roles)
        (assert (= (type configured.roles) :table)
                "config.models.roles must be an object")
        (each [role id (pairs configured.roles)]
          (assert (and (= (type role) :string) (= (type id) :string)
                       (not= id ""))
                  "model roles must map names to nonempty model IDs")))
      (when (and configured configured.catalogue_filter)
        (let [filter configured.catalogue_filter]
          (assert (= (type filter) :table)
                  "config.models.catalogue_filter must be an object")
          (each [_ key (ipairs [:max_age_days :popular_limit])]
            (let [value (. filter key)]
              (assert (or (= value nil)
                          (and (= (type value) :number) (>= value 0)))
                      "model catalogue limits must be nonnegative numbers")))))
      (let [default (or (and (= (type configured) :table)
                             (or (and configured.roles configured.roles.default)
                                 configured.default))
                        nil)]
        (when (not= default nil)
          (assert (and (= (type default) :string) (not= default ""))
                  "config.models.default must be a nonempty string"))
        (table.insert declarations
                      {:catalog :events
                       :value {:event :app/start
                               :handler (fn [db _ cofx]
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
                                              result)))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :model/picker-open
                               :handler (fn [db]
                                          (if (or db.picker db.dialog)
                                              {:fx [{:type :terminal/read}]}
                                              {:fx [{:event {:command :/model
                                                             :type :choices/command-open}
                                                     :type :dispatch}]}))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :model/open
                               :handler (fn [db event]
                                          (let [state (assert db.models
                                                              "model state is not initialized")
                                                requested (or (and (= (type event.arguments)
                                                                      :string)
                                                                   (event.arguments:match "^%s*(%S+)%s*$"))
                                                              nil)]
                                            (assert (and requested
                                                         (find state.entries
                                                               requested))
                                                    "unknown or unavailable model")
                                            {:patch {:models {:selected requested}}
                                             :fx [{:type :terminal/read}]}))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :models/provider-availability
                               :handler (fn [db event]
                                          (assert (and (= (type event.provider)
                                                          :string)
                                                       (= (type event.available)
                                                          :boolean))
                                                  "invalid provider availability")
                                          (let [state (assert db.models
                                                              "model state is not initialized")]
                                            (updated (rebuild (misa.patch state
                                                                          {:available {event.provider event.available}})
                                                              default))))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :models/update
                               :handler (fn [db event]
                                          (assert (and (= (type event.provider)
                                                          :string)
                                                       (= (type event.models)
                                                          :table))
                                                  "invalid model update")
                                          (let [state (assert db.models
                                                              "model state is not initialized")
                                                updates {}]
                                            (each [_ update (ipairs event.models)]
                                              (assert (and (= (type update)
                                                              :table)
                                                           (= (type update.id)
                                                              :string))
                                                      "invalid model update")
                                              (assert (or (= update.context_window
                                                             nil)
                                                          (and (= (type update.context_window)
                                                                  :number)
                                                               (> update.context_window
                                                                  0)
                                                               (= (% update.context_window
                                                                     1)
                                                                  0)))
                                                      "invalid context window")
                                              (tset updates update.id update))
                                            (let [catalogue {}]
                                              (each [index model (ipairs state.catalogue)]
                                                (let [update (or (and (= model.provider
                                                                         event.provider)
                                                                      (. updates
                                                                         model.id))
                                                                 nil)]
                                                  (tset catalogue index
                                                        (if update
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
                                                                default)))))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :models/replace-provider
                               :handler (fn [db event]
                                          (assert (and (= (type event.provider)
                                                          :string)
                                                       (not= event.provider ""))
                                                  "model provider must be nonempty")
                                          (assert (= (type event.models) :table)
                                                  "models must be an array")
                                          (let [(state catalogue seen) (values (assert db.models
                                                                                       "model state is not initialized")
                                                                               {}
                                                                               {})]
                                            (each [_ model (ipairs state.catalogue)]
                                              (when (or (not= model.provider
                                                              event.provider)
                                                        (and (= model.id
                                                                state.selected)
                                                             (not= event.authoritative
                                                                   true)))
                                                (tset catalogue
                                                      (+ (length catalogue) 1)
                                                      model)
                                                (tset seen model.id true)))
                                            (each [_ model (ipairs event.models)]
                                              (assert (and (= (type model)
                                                              :table)
                                                           (= (type model.id)
                                                              :string)
                                                           (not= model.id ""))
                                                      "invalid discovered model")
                                              (assert (and (= (type model.model)
                                                              :string)
                                                           (not= model.model ""))
                                                      "invalid discovered model ID")
                                              (assert (or (= model.context_window
                                                             nil)
                                                          (and (= (type model.context_window)
                                                                  :number)
                                                               (> model.context_window
                                                                  0)
                                                               (= (% model.context_window
                                                                     1)
                                                                  0)))
                                                      "invalid context window")
                                              (when (. seen model.id)
                                                (each [i existing (ipairs catalogue)]
                                                  (when (= existing.id model.id)
                                                    (table.remove catalogue i)
                                                    (lua :break))))
                                              (tset seen model.id true)
                                              (tset catalogue
                                                    (+ (length catalogue) 1)
                                                    {:api model.api
                                                     :context_window model.context_window
                                                     :id model.id
                                                     :created model.created
                                                     :recommended model.recommended
                                                     :popularity_rank model.popularity_rank
                                                     :label (or (and (= (type model.label)
                                                                        :string)
                                                                     model.label)
                                                                model.id)
                                                     :model model.model
                                                     :pricing model.pricing
                                                     :provider event.provider}))
                                            (updated (rebuild (misa.patch state
                                                                          {:catalogue (misa.replace catalogue)})
                                                              default))))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :model/select
                               :handler (fn [db event]
                                          (assert (and (= (type event.id)
                                                          :string)
                                                       (find db.models.entries
                                                             event.id))
                                                  "unknown or unavailable model")
                                          {:patch {:models {:selected event.id}}})}})
        (table.insert declarations
                      {:catalog :services
                       :id :models.for-role
                       :value (fn [db role]
                                "Resolve the model selected for a named role."
                                (let [state db.models]
                                  (when state
                                    (find state.entries
                                          (if (= role :default) state.selected
                                              (and state.roles
                                                   (. state.roles role)))))))})
        (table.insert declarations
                      (let [definition {:name :/role
                                        :description "Assign a model to a role (default or summarizer)"
                                        :event :model/role
                                        :complete (fn [_ db]
                                                    (let [items []]
                                                      (each [_ role (ipairs [:default
                                                                             :summarizer])]
                                                        (each [_ model (ipairs (or (and db.models
                                                                                        db.models.entries)
                                                                                   []))]
                                                          (table.insert items
                                                                        {:value (.. role
                                                                                    " "
                                                                                    model.id)})))
                                                      (table.insert items
                                                                    {:value "summarizer off"})
                                                      items))}]
                        {:catalog :commands
                         :id (. definition :name)
                         :value definition}))
        (table.insert declarations
                      {:catalog :events
                       :value {:event :model/role
                               :handler (fn [db event]
                                          (let [(role id) (: (or event.arguments
                                                                 "")
                                                             :match
                                                             "^(%S+)%s+(%S+)$")]
                                            (assert (and role id
                                                         (or (= role :default)
                                                             (= role
                                                                :summarizer))
                                                         (or (and (= role
                                                                     :summarizer)
                                                                  (= id :off))
                                                             (find db.models.entries
                                                                   id)))
                                                    "use /role default|summarizer provider/model (or summarizer off)")
                                            (let [roles (misa.patch (or db.models.roles
                                                                        {})
                                                                    {role (if (= id
                                                                                 :off)
                                                                              misa.delete
                                                                              id)})]
                                              {:patch {:models {:roles (misa.replace roles)
                                                                :selected (when (= role
                                                                                   :default)
                                                                            id)}}
                                               :fx [{:type :state/save
                                                     :namespace :model-roles
                                                     :data roles}
                                                    {:type :terminal/read}]})))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :model/roles-loaded
                               :handler (fn [db event]
                                          (when (and (not= event.found false)
                                                     (= (type event.data)
                                                        :table))
                                            (let [roles {}]
                                              (each [role id (pairs event.data)]
                                                (when (and (= (type role)
                                                              :string)
                                                           (= (type id) :string))
                                                  (tset roles role id)))
                                              (each [role id (pairs (or (and configured
                                                                             configured.roles)
                                                                        {}))]
                                                (tset roles role id))
                                              {:patch {:models {:roles (misa.replace roles)
                                                                :selected (when roles.default
                                                                            roles.default)}}})))}})
        (definitions.build :models declarations {})))))

{: build}
