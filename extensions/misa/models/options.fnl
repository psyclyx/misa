(local definitions (require :misa.definitions))

;; Generic per-model request-option selection and readiness. Model providers
;; declare API capabilities; pure normalization supplies reads and state updates.

(fn selected-model [db]
  (let [models db.models]
    (if (not models) nil (do
                           (each [_ model (ipairs (or models.entries {}))]
                             (when (= model.id models.selected)
                               (lua "return model")))
                           nil))))

(fn option-declarations [model]
  (let [api (and model model.api)]
    (or (and (= (type api) :table) (= (type api.request_options) :table)
             api.request_options) {})))

(fn choices [option]
  (if (not= (type option) :table) {}
      (let [source-values (or option.choices option.values)]
        (or (and (= (type source-values) :table) source-values) {}))))

(fn equivalent [left right]
  (and (= (type left) (type right)) (= left right)))

(fn contains? [source-values value]
  (if (= value nil)
      false
      (do
        (each [_ candidate (ipairs source-values)]
          (when (equivalent candidate value) (lua "return true")))
        false)))

(fn valid? [option value]
  (if (or (= value nil) (not= (type option) :table))
      false
      (let [source-values (choices option)]
        (or (= (length source-values) 0) (contains? source-values value)))))

(fn reconcile [db model]
  "Return option selections reconciled against the model's declarations."
  (let [state (or db.request_options {:values {}})
        model (or model (selected-model db))
        previous (or state.values {})
        (next-values configured) (values {} (or state.configured {}))]
    (each [name option (pairs (option-declarations model))]
      (when (and (= (type name) :string) (= (type option) :table))
        (var value (. previous name))
        (when (not (valid? option value))
          (set value (if (= state.model_id nil) (. configured name) nil))
          (when (not (valid? option value)) (set value option.default)))
        (when (valid? option value) (tset next-values name value))))
    {:configured state.configured
     :values next-values
     :model_id (and model model.id)}))

(fn prepare [db model]
  "Build validated request options for the selected model."
  (let [state (reconcile db model)]
    (let [model (or model (selected-model db))
          (result problems) (values {} {})
          serializer (or (and model (= (type model.api) :table)
                              model.api.request_options_serializer)
                         nil)]
      (each [name option (pairs (option-declarations model))]
        (if (or (not= (type name) :string) (= name "")
                (not= (type option) :table))
            (tset problems (+ (length problems) 1)
                  {:option (tostring name) :reason :invalid_declaration})
            (let [value (. state.values name)]
              (if (and (or (not= value nil) (= option.required true))
                       (not (misa.request-options.serializable? serializer name)))
                  (tset problems (+ (length problems) 1)
                        {:option name :reason :not_serializable})
                  (and (not= value nil) (valid? option value))
                  (tset result name value)
                  (= option.required true)
                  (tset problems (+ (length problems) 1)
                        {:option name
                         :reason (or (and (= value nil) :missing)
                                     :unsupported_value)})))))
      (if (= (length problems) 0) result
          (do
            (table.sort problems (fn [left right] (< left.option right.option)))
            (var (names unserializable) (values {} false))
            (each [_ item (ipairs problems)]
              (tset names (+ (length names) 1) item.option)
              (when (= item.reason :not_serializable) (set unserializable true)))
            (let [prefix (or (and unserializable
                                  "request options cannot be serialized: ")
                             "required request options are missing: ")]
              (values nil
                      {:code (or (and unserializable
                                      :unserializable_request_options)
                                 :missing_required_request_options)
                       :kind :request_readiness
                       :message (.. prefix (table.concat names ", "))
                       :missing problems
                       :model (or (and model model.id) nil)})))))))

(fn build [context]
  "Build the declarations for request options."
  (let [declarations []]
    (var config (or (and (= (type context.config) :table)
                         context.config.request_options)
                    nil))
    (set config (or (and (= (type config) :table) config) {}))
    (let [configured (or (and (= (type config.values) :table) config.values)
                         config)]
      (table.insert declarations
                    (let [definition {:id :request-options/indicator
                                      :inputs [[:db/path :models :entries]
                                               [:db/path :models :selected]
                                               [:db/path :request_options]]
                                      :compute (fn [inputs query]
                                                 (let [db {:models {:entries (. inputs
                                                                                1)
                                                                    :selected (. inputs
                                                                                 2)}
                                                           :request_options (. inputs
                                                                               3)}
                                                       value (. (reconcile db)
                                                                :values
                                                                (. query 2))]
                                                   (when (not= value nil)
                                                     (let [kind (type value)]
                                                       (if (or (= kind :boolean)
                                                               (= kind :string)
                                                               (and (= kind
                                                                       :number)
                                                                    (= value
                                                                       value)
                                                                    (< (math.abs value)
                                                                       math.huge)))
                                                           {:type (if (= kind
                                                                         :string)
                                                                      :text
                                                                      kind)
                                                            : value}
                                                           {:type :unavailable
                                                            :reason :unsupported_option_value})))))}]
                      {:catalog :subscriptions
                       :id (. definition :id)
                       :value definition}))
      (table.insert declarations
                    {:catalog :services
                     :id :request-options.choices
                     :value (fn [db name]
                              "List the values declared for a model's request option."
                              (let [option (. (option-declarations (selected-model db))
                                              name)
                                    result {}]
                                (each [_ value (ipairs (choices option))]
                                  (tset result (+ (length result) 1) value))
                                result))})
      (table.insert declarations
                    {:catalog :services
                     :id :request-options.value
                     :value (fn [db name]
                              "Return the selected value for a request option."
                              (. (reconcile db) :values name))})
      (table.insert declarations
                    {:catalog :services
                     :id :request-options.state
                     :value (fn [db]
                              "Return the model's declared options and current selections."
                              (let [state (or db.request_options {})
                                    source-values {}]
                                (each [name value (pairs (or state.values {}))]
                                  (tset source-values name value))
                                {:model_id state.model_id
                                 :values source-values}))})
      (table.insert declarations
                    {:catalog :services
                     :id :request-options.reconcile
                     :value reconcile})
      (table.insert declarations
                    {:catalog :services
                     :id :request-options.prepare
                     :value prepare})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :app/start
                             :handler (fn []
                                        {:patch {:request_options (misa.replace {: configured
                                                                                 :values {}})}
                                         :fx [{:type :dispatch
                                               :event {:type :request-options/reconcile}}]})}})
      (each [_ event-type (ipairs [:model/role
                                   :model/roles-loaded
                                   :model/open
                                   :model/select
                                   :models/provider-availability
                                   :models/update
                                   :models/replace-provider])]
        (table.insert declarations
                      {:catalog :events
                       :value {:event event-type
                               :handler (fn []
                                          {:fx [{:type :dispatch
                                                 :event {:type :request-options/reconcile}}]})}}))
      ;; Reconcile after all owners of the triggering event have committed.
      ;; Extension registration order must not select the previous model.
      (table.insert declarations
                    {:catalog :events
                     :value {:event :request-options/reconcile
                             :handler (fn [db]
                                        (when db.request_options
                                          {:patch {:request_options (misa.replace (reconcile db))}}))}})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :request-options/select
                             :handler (fn [db event]
                                        (assert (and (= (type event.name)
                                                        :string)
                                                     (not= event.name ""))
                                                "request option name must be nonempty")
                                        (let [option (. (option-declarations (selected-model db))
                                                        event.name)]
                                          (assert (= (type option) :table)
                                                  "request option is not supported by the selected model")
                                          (assert (valid? option event.value)
                                                  "request option value is not supported by the selected model")
                                          (let [state (reconcile db)]
                                            {:patch {:request_options (misa.replace (misa.patch state
                                                                                                {:values {event.name event.value}}))}})))}})
      (definitions.build :request_options declarations {}))))

{: build}
