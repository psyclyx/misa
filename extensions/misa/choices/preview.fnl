;; Preview presentation is independent of choice geometry and domain providers.
(fn line [text]
  "Create a styled preview line."
  {:spans [{: text :style :choice.preview}]})

(fn metadata [preview context]
  "Render structured preview metadata."
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
  "Validate a preview renderer."
  (assert (= (type render) :function)
          "choice preview renderer must be a function"))

(fn choices-preview [preview context]
  "Render preview data with the implementation registered for its type."
  (if (= preview nil) []
      (let [model (if (= (type preview) :string) {:type :text :value preview}
                      preview)
            kind (or model.type :metadata)
            render (assert (. (misa.catalog :choice-previews) kind)
                           (.. "unknown preview renderer: " kind))]
        (misa.layout.wrap-spans (render model context) context.columns))))

{:choice-previews choice-previews
 :choices-preview choices-preview
 :line line
 :metadata metadata}
