;; Query evaluation is independent of UI and application state ownership.
;; Each consumer owns a bounded scope. Forks isolate speculative transactions.
(fn vector-size [value]
  (assert (and (= (type value) :table) (= (getmetatable value) nil))
          "expected query vector")
  (var count 0)
  (each [key (pairs value)]
    (assert (and (= (type key) :number) (>= key 1) (= (% key 1) 0))
            "expected query vector")
    (set count (+ count 1)))
  (for [i 1 count]
    (assert (not= (. value i) nil) "query vectors cannot have holes"))
  count)

(fn query-key [query]
  "Encode a data-only query into a deterministic cache key."
  (assert (> (vector-size query) 0) "empty subscription query")
  (assert (and (= (type (. query 1)) :string) (not= (. query 1) ""))
          "query must start with an id")
  (local active {})

  (fn encode [value depth]
    (assert (< depth 128) "subscription query is too deep")
    (local kind (type value))
    (if (= kind :string) (.. :s (length value) ":" value) (= kind :boolean)
        (if value :b1 :b0) (= kind :number)
        (do
          (assert (and (= value value) (< (math.abs value) math.huge))
                  "query number must be finite")
          (.. :n (if (= value 0) :0 (string.format "%.17g" value)) ";"))
        (= kind :table)
        (do
          (assert (and (= (getmetatable value) nil) (not (. active value)))
                  "invalid or cyclic query table")
          (tset active value true)
          (local entries [])
          (each [key item (pairs value)]
            (assert (or (= (type key) :string) (= (type key) :number)
                        (= (type key) :boolean))
                    "invalid query key")
            (table.insert entries
                          (.. (encode key (+ depth 1))
                              (encode item (+ depth 1)))))
          (table.sort entries)
          (tset active value nil)
          (.. :t (length entries) ":" (table.concat entries) :e))
        (error (.. "unsupported query value: " kind))))

  (encode query 0))

(fn new-registry []
  "Create a registry of pure subscription definitions and bounded scopes."
  (local definitions {})

  (fn register [definition]
    "Validate and retain a uniquely named subscription definition."
    (assert (and (= (type definition) :table) (= (type definition.id) :string)
                 (not= definition.id ""))
            "subscription needs a nonempty id")
    (assert (not (. definitions definition.id)) "duplicate subscription")
    (local read (= (type definition.read) :function))
    (local compute (= (type definition.compute) :function))
    (assert (not= read compute)
            "subscription needs exactly one of read or compute")
    (assert (if read (= definition.compute nil) (= definition.read nil))
            "invalid subscription callback")
    (assert (if read (= definition.inputs nil)
                (or (= (type definition.inputs) :table)
                    (= (type definition.inputs) :function)))
            "computed subscriptions need explicit inputs; reads cannot declare inputs")
    (when (= (type definition.inputs) :table)
      (vector-size definition.inputs)
      (each [_ query (ipairs definition.inputs)] (query-key query)))
    (tset definitions definition.id
          {:read definition.read
           :compute definition.compute
           :inputs definition.inputs}))

  (fn scope [capacity inherited]
    (local limit (if (= capacity nil) 256 capacity))
    (assert (and (= (type limit) :number) (>= limit 1) (< limit math.huge)
                 (= (% limit 1) 0)) "invalid subscription capacity")
    (local cache {})
    (var (clock count closed evaluating) (values 0 0 false false))
    (each [key entry (pairs (or inherited {}))]
      (tset cache key entry)
      (set count (+ count 1))
      (set clock (math.max clock entry.used)))

    (fn query [db request]
      "Resolve a query while reusing values with unchanged declared inputs."
      (assert (not closed) "subscription scope is closed")
      (assert (not evaluating)
              "subscription callbacks must declare dependencies through inputs")
      (set evaluating true)
      (local staged {})
      (local active {})
      (var depth 0)

      (fn evaluate [q]
        (local key (query-key q))
        (assert (not (. active key))
                (.. "subscription dependency cycle: " (. q 1)))
        (assert (< depth 128) "subscription dependency chain is too deep")
        (local done (. staged key))
        (if done done.value (do
                              (tset active key true)
                              (set depth (+ depth 1))
                              (local definition
                                     (assert (. definitions (. q 1))
                                             (.. "unknown subscription: "
                                                 (. q 1))))
                              (local previous (. cache key))
                              (var entry nil)
                              (if definition.read
                                  (set entry
                                       {:state db
                                        :value (if (and previous
                                                        (= previous.state db))
                                                   previous.value
                                                   (definition.read db q))})
                                  (let [queries (if (= (type definition.inputs)
                                                       :function)
                                                    (definition.inputs q)
                                                    definition.inputs)
                                        n (vector-size queries)
                                        inputs {: n}]
                                    (for [i 1 n]
                                      (tset inputs i (evaluate (. queries i))))
                                    (var same
                                         (and previous previous.inputs
                                              (= previous.inputs.n n)))
                                    (when same
                                      (for [i 1 n]
                                        (when (not= (. inputs i)
                                                    (. previous.inputs i))
                                          (set same false))))
                                    ;; Previous output is an optional immutable optimization
                                    ;; hint. Compute must remain correct without it after eviction.
                                    (set entry
                                         {: inputs
                                          :value (if same previous.value
                                                     (definition.compute inputs
                                                       q
                                                       (and previous
                                                            previous.value)))})))
                              (set depth (- depth 1))
                              (tset active key nil)
                              (tset staged key entry)
                              entry.value)))

      (local (ok result) (pcall evaluate request))
      (set evaluating false)
      (when (not ok) (error result 0))
      ;; Only successful evaluations may replace or evict memoized results.
      (each [key entry (pairs staged)]
        (when (not (. cache key)) (set count (+ count 1)))
        (set clock (+ clock 1))
        (tset cache key {:state entry.state
                         :inputs entry.inputs
                         :value entry.value
                         :used clock}))
      (while (> count limit)
        (var (victim oldest) (values nil math.huge))
        (each [key entry (pairs cache)]
          (when (< entry.used oldest)
            (set (victim oldest) (values key entry.used))))
        (tset cache victim nil)
        (set count (- count 1)))
      result)

    {: query
     :fork (fn []
             "Create an independent speculative cache from this scope."
             (assert (and (not closed) (not evaluating))
                     "subscription scope is closed or evaluating")
             (scope limit cache))
     :size (fn [] "Return the number of retained query entries." count)
     :clear (fn []
              "Discard cached queries while keeping the scope usable."
              (assert (not evaluating) "subscription scope is evaluating")
              (each [key (pairs cache)] (tset cache key nil))
              (set count 0))
     :close (fn []
              "Discard cached queries and permanently close the scope."
              (assert (not evaluating) "subscription scope is evaluating")
              (each [key (pairs cache)] (tset cache key nil))
              (set count 0)
              (set closed true))})

  {: register
   :scope (fn [capacity] "Create a cache with an optional entry limit."
            (scope capacity))
   :key query-key})

new-registry
