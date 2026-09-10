;; Generic per-model request-option selection and readiness. Model providers
;; declare API capabilities; pure normalization supplies reads and state updates.

(fn selected-model [db]
  (when db.models
    (accumulate [found nil _ model (ipairs (or db.models.entries {}))
                 &until found]
      (when (= model.id db.models.selected) model))))

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
  (and (not= value nil) (accumulate [found false _ candidate (ipairs source-values)
                                     &until found]
                          (equivalent candidate value))))

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
        next-values {}
        configured (or state.configured {})]
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
  (let [state (reconcile db model)
        model (or model (selected-model db))
        result {}
        problems {}
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
          (let [names {}]
            (var unserializable false)
            (each [_ item (ipairs problems)]
              (tset names (+ (length names) 1) item.option)
              (when (= item.reason :not_serializable)
                (set unserializable true)))
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

(fn compute-request-options-indicator [inputs query]
  "Project the selected request option indicator."
  (let [db {:models {:entries (. inputs 1) :selected (. inputs 2)}
            :request_options (. inputs 3)}
        value (. (reconcile db) :values (. query 2))]
    (when (not= value nil)
      (let [kind (type value)]
        (if (or (= kind :boolean) (= kind :string)
                (and (= kind :number) (= value value)
                     (< (math.abs value) math.huge)))
            {:type (if (= kind :string) :text
                       kind)
             : value}
            {:type :unavailable :reason :unsupported_option_value})))))

(fn request-options-choices [db name]
  "List the values declared for a model's request option."
  (let [option (. (option-declarations (selected-model db)) name)
        result {}]
    (each [_ value (ipairs (choices option))]
      (tset result (+ (length result) 1) value))
    result))

(fn request-options-state [db]
  "Return the model's declared options and current selections."
  (let [state (or db.request_options {})
        source-values {}]
    (each [name value (pairs (or state.values {}))]
      (tset source-values name value))
    {:model_id state.model_id :values source-values}))

(fn on-request-options-reconcile [db]
  "Reconcile request options with the selected model."
  (when db.request_options
    {:patch {:request_options (misa.replace (reconcile db))}}))

(fn on-request-options-select [db event]
  "Validate and update a selected request option."
  (assert (and (= (type event.name) :string) (not= event.name ""))
          "request option name must be nonempty")
  (let [option (. (option-declarations (selected-model db)) event.name)]
    (assert (= (type option) :table)
            "request option is not supported by the selected model")
    (assert (valid? option event.value)
            "request option value is not supported by the selected model")
    (let [state (reconcile db)]
      {:patch {:request_options (misa.replace (misa.patch state
                                                          {:values {event.name event.value}}))}})))

(fn schedule-reconcile []
  "Schedule request-option reconciliation after model changes."
  {:fx [{:type :dispatch :event {:type :request-options/reconcile}}]})

(fn on-app-start [config]
  "Initialize configured request options and reconcile model support."
  (let [value (or config.request_options {})
        configured (or (and (= (type value.values) :table) value.values) value)]
    {:patch {:request_options (misa.replace {: configured :values {}})}
     :fx [{:type :dispatch :event {:type :request-options/reconcile}}]}))

{: compute-request-options-indicator
 : on-app-start
 : schedule-reconcile
 : on-request-options-reconcile
 : on-request-options-select
 : prepare
 : reconcile
 : request-options-choices
 : request-options-state}
