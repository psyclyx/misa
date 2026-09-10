(fn line [text]
  {:spans [{: text :style :choice.preview}]})

(local weekday-names [:Sun :Mon :Tue :Wed :Thu :Fri :Sat])

(fn hour-text [hour]
  (string.format "%02d:00" hour))

(fn days-text [weekdays]
  "Render weekday numbers as contiguous name ranges."
  (let [names (icollect [_ day (ipairs weekdays)]
                (or (. weekday-names day) "?"))
        groups []]
    (var index 1)
    (while (<= index (length names))
      (let [first index]
        (while (and (< index (length weekdays))
                    (= (. weekdays (+ index 1)) (+ (. weekdays index) 1)))
          (set index (+ index 1)))
        (table.insert groups
                      (if (= first index) (. names index)
                          (.. (. names first) "–" (. names index)))))
      (set index (+ index 1)))
    (table.concat groups ", ")))

(fn peak-note [peak]
  "Describe the windows that multiply a model's base prices."
  (let [windows (icollect [_ window (ipairs (or peak.windows []))]
                  (.. (hour-text window.start_hour) "–"
                      (hour-text window.end_hour)))]
    (.. "Peak " (string.format "%.6g" peak.multiplier) "× at "
        (table.concat windows ", ") " UTC"
        (if peak.weekdays (.. " on " (days-text peak.weekdays)) ""))))

(fn model-preview [model context]
  "Render model capabilities and pricing as preview data."
  (let [cost model.cost
        pricing (and cost cost.pricing)
        currency (or (and cost cost.currency) :USD)
        unit (or (and cost cost.token_unit) 1000000)
        unit-text (if (= unit 1000000) :1M (tostring unit))]
    (fn rate [value]
      (if (= value nil)
          "?"
          (do
            (assert (and (= (type value) :number) (<= 0 value)
                         (< value math.huge))
                    "invalid pricing rate")
            (.. (if (= currency :USD) "$" (.. currency " "))
                (string.format "%.6g" value)))))

    (let [available (and pricing (not cost.unavailable))
          summary (if available
                      (.. (rate pricing.input) " in / " (rate pricing.output)
                          " out per " unit-text)
                      "cost unknown")]
      (if context.compact [(line summary)]
          (let [result []]
            (when model.title (table.insert result (line model.title)))
            (when model.context_window
              (table.insert result
                            (line (.. "Context: " model.context_window
                                      " tokens"))))
            (if available
                (do
                  (table.insert result
                                (line (.. currency " per " unit-text
                                          " tokens: " (rate pricing.input)
                                          " input · " (rate pricing.output)
                                          " output")))
                  (when (or (not= pricing.cache_read nil)
                            (not= pricing.cache_write nil))
                    (table.insert result
                                  (line (.. "Cache: " (rate pricing.cache_read)
                                            " read · "
                                            (rate pricing.cache_write)
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
                  (when cost.peak
                    (table.insert result (line (peak-note cost.peak))))
                  (table.insert result
                                (line "Estimates; reported usage cost takes precedence")))
                (table.insert result
                              (line "Cost: unavailable; configure costs.models for this model")))
            result)))))

{: model-preview}
