;; Preview presentation is independent of choice geometry and domain providers.
(fn line [text] {:spans [{: text :style :choice.preview}]})
(fn metadata [preview context]
  (if (and context.compact preview.summary) [(line (tostring preview.summary))]
      (let [body (if (= (type preview.lines) :table)
                     (icollect [_ text (ipairs preview.lines)] (line (tostring text)))
                     (let [keys (icollect [key (pairs preview)]
                                  (when (and (not= key :title) (not= key :type)) key))]
                       (table.sort keys (fn [a b] (< (tostring a) (tostring b))))
                       (icollect [_ key (ipairs keys)]
                         (line (.. (tostring key) ": " (tostring (. preview key)))))))]
        (if preview.title
            (let [result [(line (tostring preview.title))]]
              (each [_ item (ipairs body)] (table.insert result item)) result)
            body))))
(fn model-preview [model context]
  (local cost model.cost)
  (local pricing (and cost cost.pricing))
  (local currency (or (and cost cost.currency) :USD))
  (local unit (or (and cost cost.token_unit) 1000000))
  (local unit-text (if (= unit 1000000) "1M" (tostring unit)))
  (fn rate [value]
    (if (= value nil) "?"
        (do (assert (and (= (type value) :number) (<= 0 value) (< value math.huge)) "invalid pricing rate")
            (.. (if (= currency :USD) "$" (.. currency " ")) (string.format "%.6g" value)))))
  (local available (and pricing (not cost.unavailable)))
  (local summary (if available
                    (.. (rate pricing.input) " in / " (rate pricing.output) " out per " unit-text)
                    "cost unknown"))
  (if context.compact [(line summary)]
      (let [result []]
        (when model.title (table.insert result (line model.title)))
        (when model.context_window
          (table.insert result (line (.. "Context: " model.context_window " tokens"))))
        (if available
            (do
              (table.insert result (line (.. currency " per " unit-text " tokens: "
                                             (rate pricing.input) " input · " (rate pricing.output) " output")))
              (when (or (not= pricing.cache_read nil) (not= pricing.cache_write nil))
                (table.insert result (line (.. "Cache: " (rate pricing.cache_read) " read · "
                                               (rate pricing.cache_write) " write per " unit-text))))
              (when (and pricing.request (> pricing.request 0))
                (table.insert result {:spans (let [spans [{:text "Per request: " :style :choice.preview}]]
                                               (each [_ span (ipairs (misa.render_value {:type :money :amount pricing.request
                                                                                       : currency}))]
                                                 (table.insert spans span)) spans)}))
              (table.insert result (line "Estimates; reported usage cost takes precedence")))
            (table.insert result (line "Cost: unavailable; configure costs.models for this model")))
        result)))
{:setup (fn [context]
          (local renderers {})
          (local selected (or (. (or (. (or context.config {}) :choices) {}) :preview_renderers) {}))
          {:fx [{:type :register/setup-effect :name :register/choice-preview
                 :handler (fn [effect]
                            (assert (and (= (type effect.id) :string) (not= effect.id "")
                                         (= (type effect.render) :function)) "preview renderer requires id and render")
                            (assert (= (. renderers effect.id) nil) "duplicate preview renderer")
                            (tset renderers effect.id effect.render) nil)}
                {:type :register/choice-preview :id :metadata :render metadata}
                {:type :register/choice-preview :id :model :render model-preview}
                {:type :register/choice-preview :id :text :render (fn [preview] [(line preview.value)])}
                {:type :register/service :name :choice_preview
                 :value (fn [preview context]
                          (if (= preview nil) []
                              (let [model (if (= (type preview) :string) {:type :text :value preview} preview)
                                    kind (or model.type :metadata)
                                    id (or (. selected kind) kind)
                                    render (assert (. renderers id) (.. "unknown preview renderer: " id))]
                                (misa.layout.wrap_spans (render model context) context.columns))))}]})}
