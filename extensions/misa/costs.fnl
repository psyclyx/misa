;; Optional monetary accounting. Providers supply prices and normalized usage;
;; presentation consumes projections. No UI or provider identity is required.

(local rate-names [:input :output :cache_read :cache_write :request])

(fn valid? [value]
  (and (= (type value) :number) (>= value 0) (< value math.huge)))

(fn rates [value]
  (if (not= (type value) :table) nil
      (let [result {}]
        (each [_ name (ipairs rate-names)]
          (when (not= (. value name) nil)
            (assert (valid? (. value name))
                    (.. "cost rate " name
                        " must be a finite nonnegative number"))
            (tset result name (. value name))))
        (or (and (next result) result) nil))))

(fn estimate [pricing usage]
  "Estimate a request's cost from token prices and reported usage."
  (if (valid? usage.cost_usd)
      {:estimated false :unknown false :usd usage.cost_usd}
      (not pricing)
      {:estimated true :unknown true :usd 0}
      (let [read (or usage.cache_read_tokens 0)
            write (or usage.cache_write_tokens 0)
            input (or usage.input_tokens 0)
            counts {:cache_read read
                    :cache_write write
                    :input (if usage.input_includes_cache
                               (math.max 0 (- input read write))
                               input)
                    :output (or usage.output_tokens 0)}
            result {:estimated true :unknown false :usd (or pricing.request 0)}]
        (each [name count (pairs counts)]
          (if (not (valid? count)) (set result.unknown true) (< 0 count)
              (if (= (. pricing name) nil) (set result.unknown true)
                  (set result.usd
                       (+ result.usd (/ (* count (. pricing name)) 1000000))))))
        result)))

(fn model-rates [overrides db id]
  (if (. overrides id) (. overrides id)
      (let [model (accumulate [found nil _ model (ipairs (or (and db.models
                                                                  db.models.catalogue)
                                                             (misa.models.all)))
                               &until found]
                    (when (= model.id id) model))]
        (when model (rates model.pricing)))))

(fn costs-model [overrides db id]
  "Return configured pricing and availability for a model."
  (let [pricing (model-rates overrides db id)]
    {:currency :USD
     :token_unit 1000000
     : pricing
     :estimated true
     :unavailable (= pricing nil)}))

(fn costs-responses [inputs _ previous]
  "Project response accounting into monetary facts."
  (let [entries {}]
    (each [id response (pairs (or (. inputs 1) {}))]
      (let [old (and previous (. previous id))]
        (if (and old (= response old.input))
            (tset entries id old)
            (let [result response.cost]
              (tset entries id
                    {:input response
                     :value {:type :money
                             :currency :USD
                             :pending (= result nil)
                             :model response.model
                             :estimated (= (and result result.estimated) true)
                             :unknown (= (and result result.unknown) true)
                             :amount (and result result.usd)}})))))
    entries))

(fn response-value [inputs query]
  "Select the monetary fact for one response."
  (let [entry (. (. inputs 1) (. query 2))]
    (and entry entry.value)))

(fn costs-groups [inputs _ previous]
  "Aggregate child request costs by response group."
  (let [groups {}]
    (each [id response (pairs (or (. inputs 1) {}))]
      (let [owner (or response.parent_response_id id)
            group (or (. groups owner)
                      {:type :money
                       :currency :USD
                       :amount 0
                       :estimated false
                       :unknown false
                       :pending true})
            cost response.cost]
        (when cost
          (set group.pending false)
          (set group.amount (+ group.amount cost.usd))
          (set group.estimated (or group.estimated cost.estimated false))
          (set group.unknown (or group.unknown cost.unknown false)))
        (tset groups owner group)))
    (each [id group (pairs groups)]
      (when group.pending
        (set group.amount nil))
      (let [old (and previous (. previous id))]
        (when (and old (= old.amount group.amount)
                   (= old.estimated group.estimated)
                   (= old.unknown group.unknown) (= old.pending group.pending))
          (tset groups id old))))
    groups))

(fn costs-group [db id]
  "Aggregate cost facts for the supplied response IDs."
  (. (misa.sub db [:costs/groups]) id))

(fn costs-total [inputs]
  "Aggregate all completed response costs."
  (let [total {:estimated false :unknown false :usd 0}]
    (var count 0)
    (each [_ response (pairs (or (. inputs 1) {}))]
      (when response.cost
        (set count (+ count 1))
        (set total.usd (+ total.usd response.cost.usd))
        (set total.estimated (or total.estimated response.cost.estimated))
        (set total.unknown (or total.unknown response.cost.unknown))))
    (set total.responses count)
    total))

(fn costs-indicator [inputs]
  "Describe the total as a monetary indicator."
  (let [total (. inputs 1)]
    {:type :money
     :amount total.usd
     :currency :USD
     :estimated total.estimated
     :unknown total.unknown}))

(fn costs-response [db id]
  "Return cost facts for a response."
  (let [entry (. (misa.sub db [:costs/responses]) id)]
    (and entry entry.value)))

(fn reset [_ _event]
  "Initialize empty response accounting."
  {:costs (misa.replace {:responses {}})})

(fn response-patch [id response]
  {:costs {:responses {id (misa.replace response)}}})

(fn complete [overrides db event]
  "Record the reported or estimated cost of a completed response."
  (let [response (or (. db.costs.responses event.response_id)
                     (when event.model
                       {:model event.model
                        :pricing (model-rates overrides db event.model)}))]
    (when response
      (let [usage (misa.patch (or event.usage {}) {:cost_usd event.cost_usd})]
        (response-patch event.response_id
                        (misa.patch response
                                    {:parent_response_id event.parent_response_id
                                     :call_id event.call_id
                                     :cost (misa.replace (estimate response.pricing
                                                                   usage))}))))))

(fn start-response [overrides db event]
  "Capture pricing when a response starts."
  (when (and db.costs event.model)
    (response-patch event.response_id
                    {:model event.model
                     :pricing (model-rates overrides db event.model)})))

(fn interrupt-response [db event]
  "Record known usage or an unknown cost for an interrupted response."
  (let [response (and db.costs (. db.costs.responses event.response_id))]
    (when response
      (response-patch event.response_id
                      (misa.patch response
                                  {:cost (misa.replace (if event.usage
                                                           (estimate response.pricing
                                                                     event.usage)
                                                           {:estimated true
                                                            :unknown true
                                                            :usd 0}))})))))

(fn overrides [config]
  "Validate model-specific price overrides."
  (collect [id value (pairs (or config.models {}))]
    (do
      (assert (= (type id) :string) "cost model ID must be a string")
      id)
    (rates value)))

{: estimate
 : costs-model
 : costs-responses
 : response-value
 : costs-groups
 : costs-group
 : costs-total
 : costs-indicator
 : costs-response
 : reset
 : complete
 : start-response
 : interrupt-response
 : overrides}
