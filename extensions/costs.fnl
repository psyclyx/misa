;; Optional monetary accounting. Providers supply prices and normalized usage;

;; presentation consumes projections. No UI or provider identity is required.

(local rate-names [:input :output :cache_read :cache_write :request])

(fn valid [value]
  (and (and (= (type value) :number) (>= value 0)) (< value math.huge)))

(fn rates [value]
  (if (not= (type value) :table) nil
      (let [result {}]
        (each [_ name (ipairs rate-names)]
          (when (not= (. value name) nil)
            (assert (valid (. value name))
                    (.. "cost rate " name
                        " must be a finite nonnegative number"))
            (tset result name (. value name))))
        (or (and (next result) result) nil))))

(fn dollars [value]
  (if (= value 0)
      :$0
      (if (< value 0.0001)
          :<$0.0001
          (.. "$" (string.format (or (and (< value 1) "%.4f") "%.2f") value)))))

(fn rate [value]
  (or (and (not= value nil) (.. "$" (string.format "%.6g" value))) "?"))

(fn estimate [pricing usage]
  (if (valid usage.cost_usd) {:estimated false
                              :unknown false
                              :usd usage.cost_usd}
      (if (not pricing) {:estimated true :unknown true :usd 0}
          (do
            (var (input read write)
                 (values (or usage.input_tokens 0)
                         (or usage.cache_read_tokens 0)
                         (or usage.cache_write_tokens 0)))
            (when usage.input_includes_cache
              (set input (math.max 0 (- (- input read) write))))
            (local counts
                   {:cache_read read
                    :cache_write write
                    : input
                    :output (or usage.output_tokens 0)})
            (local result
                   {:estimated true :unknown false :usd (or pricing.request 0)})
            (each [name count (pairs counts)]
              (if (not (valid count)) (set result.unknown true) (> count 0)
                  (if (= (. pricing name) nil) (set result.unknown true)
                      (set result.usd
                           (+ result.usd (/ (* count (. pricing name)) 1000000))))))
            result))))

(fn label [value]
  (if (not value)
      "cost pending"
      (if (and value.unknown (= value.usd 0))
          "cost ?"
          (.. (or (and value.estimated "~") "") (dollars value.usd)
              (or (and value.unknown " + ?") "")))))

{:setup (fn [context]
          (local setup-fx [])
          (var config (or (and (= (type context.config) :table)
                               context.config.costs)
                          {}))
          (set config (or (and (= (type config) :table) config) {}))
          (local overrides {})
          (each [id value (pairs (or config.models {}))]
            (assert (= (type id) :string) "cost model ID must be a string")
            (tset overrides id (rates value)))

          (fn model-rates [db id]
            (if (. overrides id) (. overrides id)
                (do
                  (each [_ model (ipairs (or (and db.models db.models.catalogue)
                                             (misa.models)))]
                    (when (= model.id id)
                      (let [___antifnl_rtns_1___ [(rates model.pricing)]]
                        (lua "return (table.unpack or _G.unpack)(___antifnl_rtns_1___)"))))
                  nil)))

          (table.insert setup-fx
                        {:type :register/service
                         :name :cost_estimate
                         :value estimate})
          (table.insert setup-fx
                        {:type :register/service
                         :name :cost_format
                         :value label})
          (table.insert setup-fx
                        {:type :register/service
                         :name :model_cost_info
                         :value (fn [db id]
                                  (local pricing (model-rates db id))
                                  (if (not pricing)
                                      {:lines ["Cost: unavailable; configure costs.models for this model"]
                                       :summary "cost unknown"}
                                      (do
                                        (local summary
                                               (.. (rate pricing.input)
                                                   " in / "
                                                   (rate pricing.output)
                                                   " out per 1M"))
                                        (local lines
                                               [(.. "USD per 1M tokens: "
                                                    (rate pricing.input)
                                                    " input · "
                                                    (rate pricing.output)
                                                    " output")])
                                        (when (or (not= pricing.cache_read nil)
                                                  (not= pricing.cache_write nil))
                                          (tset lines (+ (length lines) 1)
                                                (.. "Cache: "
                                                    (rate pricing.cache_read)
                                                    " read · "
                                                    (rate pricing.cache_write)
                                                    " write per 1M")))
                                        (when (and pricing.request
                                                   (> pricing.request 0))
                                          (tset lines (+ (length lines) 1)
                                                (.. "Per request: "
                                                    (dollars pricing.request))))
                                        (tset lines (+ (length lines) 1)
                                              "Estimates; reported usage cost takes precedence")
                                        {: lines : pricing : summary})))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :response_cost_projection
                         :value (fn [db id]
                                  (local response
                                         (and db.costs
                                              (. db.costs.responses id)))
                                  (if (not response) nil
                                      (do
                                        (local result response.cost)
                                        {:estimated (and result
                                                         result.estimated)
                                         :model response.model
                                         :text (label result)
                                         :unknown (and result result.unknown)
                                         :usd (and result result.usd)})))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :costs_projection
                         :value (fn [db]
                                  (local state db.costs)
                                  (local total
                                         {:estimated false
                                          :unknown false
                                          :usd 0})
                                  (var count 0)
                                  (each [_ response (pairs (or (and state
                                                                    state.responses)
                                                               {}))]
                                    (when response.cost (set count (+ count 1))
                                      (set total.usd
                                           (+ total.usd response.cost.usd))
                                      (set total.estimated
                                           (or total.estimated
                                               response.cost.estimated))
                                      (set total.unknown
                                           (or total.unknown
                                               response.cost.unknown))))
                                  (set total.text (label total))
                                  (set total.responses count)
                                  total)})
          (when (misa.has_setup_effect :register/indicator)
            (table.insert setup-fx
                          {:type :register/indicator
                           :value {:icon "$"
                                   :id :cost
                                   :label :cost
                                   :value (fn [db]
                                            (. (misa.costs_projection db) :text))}}))
          (fn reset [_ _event]
            {:costs (misa.replace {:responses {}})})
          (fn response-patch [id response]
            {:costs {:responses {id (misa.replace response)}}})
          (fn complete [db event]
            (local response (or (. db.costs.responses event.response_id)
                                (when event.model
                                  {:model event.model
                                   :pricing (model-rates db event.model)})))
            (when response
              (local usage (misa.patch (or event.usage {})
                                      {:cost_usd event.cost_usd}))
              (response-patch event.response_id
                              (misa.patch response
                                          {:cost (misa.replace
                                                   (estimate response.pricing usage))}))))
          (local transitions
                 {:app/start reset
                  :transcript/reset reset
                  :transcript/response-start
                  (fn [db event]
                    (when (and db.costs event.model)
                      (response-patch event.response_id
                                      {:model event.model
                                       :pricing (model-rates db event.model)})))
                  :transcript/response-end
                  (fn [db event] (when db.costs (complete db event)))
                  :transcript/response-interrupted
                  (fn [db event]
                    (local response (and db.costs (. db.costs.responses event.response_id)))
                    (when response
                      (response-patch event.response_id
                                      (misa.patch response
                                                  {:cost (misa.replace
                                                           (if event.usage
                                                               (estimate response.pricing event.usage)
                                                               {:estimated true :unknown true :usd 0}))}))))})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (local transition (. transitions tx.event.type))
                                           (local patch (and transition (transition tx.db tx.event)))
                                           (when patch
                                             (set tx.db (misa.patch tx.db patch)))
                                           tx)
                                 :id :costs/account}})
          {:fx setup-fx})}
