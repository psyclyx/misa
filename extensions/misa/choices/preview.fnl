(local definitions (require :misa.definitions))

;; Preview presentation is independent of choice geometry and domain providers.
(fn line [text]
  {:spans [{: text :style :choice.preview}]})

(fn metadata [preview context]
  (if (and context.compact preview.summary) [(line (tostring preview.summary))]
      (let [body (if (= (type preview.lines) :table)
                     (icollect [_ text (ipairs preview.lines)]
                       (line (tostring text)))
                     (let [keys (icollect [key (pairs preview)]
                                  (when (and (not= key :title) (not= key :type))
                                    key))]
                       (table.sort keys
                                   (fn [a b] (< (tostring a) (tostring b))))
                       (icollect [_ key (ipairs keys)]
                         (line (.. (tostring key) ": "
                                   (tostring (. preview key)))))))]
        (if preview.title
            (let [result [(line (tostring preview.title))]]
              (each [_ item (ipairs body)] (table.insert result item))
              result)
            body))))

(fn choice-previews [_ render]
  (assert (= (type render) :function)
          "choice preview renderer must be a function"))

(fn build [context]
  "Build the declarations for choice preview."
  (let [selected (or (. (or (. (or context.config {}) :choices) {})
                        :preview_renderers) {})]
    (fn choices-preview [preview context]
      "Render typed preview data using its selected implementation."
      (if (= preview nil) []
          (let [model (if (= (type preview) :string)
                          {:type :text :value preview}
                          preview)
                kind (or model.type :metadata)
                id (or (. selected kind) kind)
                render (assert (. (misa.catalog :choice-previews) id)
                               (.. "unknown preview renderer: " id))]
            (misa.layout.wrap-spans (render model context) context.columns))))

    (definitions.build :choice_preview
      [{:catalog :choice-previews :id :metadata :value metadata}
       {:catalog :choice-previews
        :id :text
        :value (fn [preview] [(line preview.value)])}
       {:catalog :services :id :choices.preview :value choices-preview}]
      {:validators {:choice-previews choice-previews}})))

{: build}
