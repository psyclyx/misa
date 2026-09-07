;; Generic per-model request-option selection and readiness. Model providers

;; declare API capabilities; pure normalization supplies reads and state updates.

(fn selected-model [db]
  (let [models db.models]
    (if (not models) nil (do
                           (each [_ model (ipairs (or models.entries {}))]
                             (when (= model.id models.selected)
                               (lua "return model")))
                           nil))))

(fn declarations [model]
  (let [api (and model model.api)]
    (or (and (and (= (type api) :table) (= (type api.request_options) :table))
             api.request_options) {})))

(fn choices [option]
  (if (not= (type option) :table) {}
      (let [___values___ (or option.choices option.values)]
        (or (and (= (type ___values___) :table) ___values___) {}))))

(fn equivalent [left right]
  (and (= (type left) (type right)) (= left right)))

(fn contains [___values___ value]
  (if (= value nil)
      false
      (do
        (each [_ candidate (ipairs ___values___)]
          (when (equivalent candidate value) (lua "return true")))
        false)))

(fn valid [option value]
  (if (or (= value nil) (not= (type option) :table))
      false
      (let [___values___ (choices option)]
        (or (= (length ___values___) 0) (contains ___values___ value)))))

(fn reconcile [db model]
  (local state (or db.request_options {:values {}}))
  (set-forcibly! model (or model (selected-model db)))
  (local previous (or state.values {}))
  (local (next-values configured) (values {} (or state.configured {})))
  (each [name option (pairs (declarations model))]
    (when (and (= (type name) :string) (= (type option) :table))
      (var value (. previous name))
      (when (not (valid option value))
        (set value (if (= state.model_id nil) (. configured name) nil))
        (when (not (valid option value)) (set value option.default)))
      (when (valid option value) (tset next-values name value))))
  {:configured state.configured
   :values next-values
   :model_id (and model model.id)})

(fn prepare [db model]
  (let [state (reconcile db model)]
    (set-forcibly! model (or model (selected-model db)))
    (local (result problems) (values {} {}))
    (local serializer (or (and (and model (= (type model.api) :table))
                               model.api.request_options_serializer)
                          nil))
    (each [name option (pairs (declarations model))]
      (if (or (or (not= (type name) :string) (= name ""))
              (not= (type option) :table))
          (tset problems (+ (length problems) 1)
                {:option (tostring name) :reason :invalid_declaration})
          (let [value (. state.values name)]
            (if (and (or (not= value nil) (= option.required true))
                     (not (misa.can_serialize_request_option serializer name)))
                (tset problems (+ (length problems) 1)
                      {:option name :reason :not_serializable})
                (and (not= value nil) (valid option value))
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
          (local prefix
                 (or (and unserializable
                          "request options cannot be serialized: ")
                     "required request options are missing: "))
          (values nil
                  {:code (or (and unserializable
                                  :unserializable_request_options)
                             :missing_required_request_options)
                   :kind :request_readiness
                   :message (.. prefix (table.concat names ", "))
                   :missing problems
                   :model (or (and model model.id) nil)})))))

{:setup (fn [context]
          (local setup-fx [])
          (var config (or (and (= (type context.config) :table)
                               context.config.request_options)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local configured (or (and (= (type config.values) :table)
                                     config.values)
                                config))
          (table.insert setup-fx
                        {:type :register/sub
                         :value {:id :request-options/indicator
                                 :inputs [[:db/path :models :entries] [:db/path :models :selected]
                                          [:db/path :request_options]]
                                 :compute (fn [inputs query]
                                            (local db {:models {:entries (. inputs 1) :selected (. inputs 2)}
                                                       :request_options (. inputs 3)})
                                            (local value (. (reconcile db) :values (. query 2)))
                                            (when (not= value nil)
                                              (local kind (type value))
                                              (if (or (= kind :boolean) (= kind :string)
                                                      (and (= kind :number) (= value value)
                                                           (< (math.abs value) math.huge)))
                                                  {:type (if (= kind :string) :text kind) : value}
                                                  {:type :unavailable :reason :unsupported_option_value})))}})
          (table.insert setup-fx
                        {:type :register/service
                         :name :request_option_choices
                         :value (fn [db name]
                                  (local option
                                         (. (declarations (selected-model db))
                                            name))
                                  (local result {})
                                  (each [_ value (ipairs (choices option))]
                                    (tset result (+ (length result) 1) value))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :request_option_value
                         :value (fn [db name]
                                  (. (reconcile db) :values name))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :request_options_projection
                         :value (fn [db]
                                  (local state (or db.request_options {}))
                                  (local ___values___ {})
                                  (each [name value (pairs (or state.values {}))]
                                    (tset ___values___ name value))
                                  {:model_id state.model_id
                                   :values ___values___})})
          (table.insert setup-fx
                        {:type :register/service
                         :name :reconcile_request_options
                         :value reconcile})
          (table.insert setup-fx
                        {:type :register/service
                         :name :prepare_request_options
                         :value prepare})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:patch {:request_options
                                             (misa.replace {: configured :values {}})}
                                     :fx [{:type :dispatch :event {:type :request-options/reconcile}}]})})
          (each [_ event-type (ipairs [:model/open
                                       :model/select
                                       :models/provider-availability
                                       :models/update
                                       :models/replace-provider])]
            (table.insert setup-fx
                          {:type :register/event
                           :name event-type
                           :handler (fn []
                                      {:fx [{:type :dispatch
                                             :event {:type :request-options/reconcile}}]})}))
          ;; Reconcile after all owners of the triggering event have committed.
          ;; Extension registration order must not select the previous model.
          (table.insert setup-fx
                        {:type :register/event
                         :name :request-options/reconcile
                         :handler (fn [db]
                                    (when db.request_options
                                      {:patch {:request_options
                                               (misa.replace (reconcile db))}}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :request-options/select
                         :handler (fn [db event]
                                    (assert (and (= (type event.name) :string)
                                                 (not= event.name ""))
                                            "request option name must be nonempty")
                                    (local option
                                           (. (declarations (selected-model db))
                                              event.name))
                                    (assert (= (type option) :table)
                                            "request option is not supported by the selected model")
                                    (assert (valid option event.value)
                                            "request option value is not supported by the selected model")
                                    (local state (reconcile db))
                                    {:patch {:request_options
                                             (misa.replace
                                               (misa.patch state
                                                           {:values {event.name event.value}}))}})})
          {:fx setup-fx})}
