(local definitions (require :misa.definitions))

(fn line [text]
  {:spans [{: text :style :choice.preview}]})

(fn model-preview [model context]
  (local cost model.cost)
  (local pricing (and cost cost.pricing))
  (local currency (or (and cost cost.currency) :USD))
  (local unit (or (and cost cost.token_unit) 1000000))
  (local unit-text (if (= unit 1000000) "1M" (tostring unit)))

  (fn rate [value]
    (if (= value nil) "?"
        (do
          (assert (and (= (type value) :number) (<= 0 value)
                       (< value math.huge))
                  "invalid pricing rate")
          (.. (if (= currency :USD) "$" (.. currency " "))
              (string.format "%.6g" value)))))

  (local available (and pricing (not cost.unavailable)))
  (local summary (if available
                     (.. (rate pricing.input) " in / " (rate pricing.output)
                         " out per " unit-text)
                     "cost unknown"))
  (if context.compact [(line summary)]
      (let [result []]
        (when model.title (table.insert result (line model.title)))
        (when model.context_window
          (table.insert result
                        (line (.. "Context: " model.context_window " tokens"))))
        (if available
            (do
              (table.insert result
                            (line (.. currency " per " unit-text " tokens: "
                                      (rate pricing.input) " input · "
                                      (rate pricing.output) " output")))
              (when (or (not= pricing.cache_read nil)
                        (not= pricing.cache_write nil))
                (table.insert result
                              (line (.. "Cache: " (rate pricing.cache_read)
                                        " read · " (rate pricing.cache_write)
                                        " write per " unit-text))))
              (when (and pricing.request (> pricing.request 0))
                (table.insert result
                              {:spans (let [spans [{:text "Per request: "
                                                    :style :choice.preview}]]
                                        (each [_ span (ipairs (misa.values.render {:type :money
                                                                                   :amount pricing.request
                                                                                   : currency}))]
                                          (table.insert spans span))
                                        spans)}))
              (table.insert result
                            (line "Estimates; reported usage cost takes precedence")))
            (table.insert result
                          (line "Cost: unavailable; configure costs.models for this model")))
        result)))

(fn []
  "Declare the model preview renderer."
  (definitions :models.preview
    [{:catalog :choice-previews :id :model :value model-preview}]
    {:requirements {:models.preview [:values.render]}}))
