;; Content primitives accept ordinary values; callers choose their application.
(fn text [model context]
  "Render plain text with optional source coordinates."
  (let [lines (misa.markdown.view.plain (tostring (or model.text ""))
                                        (or model.style :plain))]
    (when (= model.source false)
      (each [_ line (ipairs lines)]
        (set line.source_start nil)
        (set line.source_end nil)
        (each [_ part (ipairs line.spans)]
          (set part.source false)
          (set part.source_start nil)
          (set part.source_end nil))))
    {:lines (misa.layout.wrap-spans lines (math.max 1 (or context.columns 80)))}))

(fn fields [model context]
  "Render labeled fields using the shared text layout."
  (let [lines []]
    (each [_ field (ipairs (or model.fields []))]
      (let [rows (. (text {:text field.value :source false :style model.style}
                          context) :lines)
            label {:text (.. (tostring field.label) "  ")
                   :style :dim
                   :source false}]
        (when (. rows 1) (table.insert (. rows 1 :spans) 1 label))
        (each [_ row (ipairs rows)] (table.insert lines row))))
    {:lines (misa.layout.wrap-spans lines (math.max 1 (or context.columns 80)))}))

(fn code [model context language]
  "Render source rows using Markdown code-block geometry."
  (let [lines (misa.markdown.view.render {:blocks [{:kind :code_block
                                                    :language (or language
                                                                  model.language)
                                                    :numbered model.numbered
                                                    :rows model.rows
                                                    :missing_newline model.missing_newline
                                                    :terminated true
                                                    :text (tostring (or model.text
                                                                        ""))
                                                    :source_start 0
                                                    :content_start 0}]}
                                         {:base (or model.style :plain)
                                          :columns (math.max 1
                                                             (or context.columns
                                                                 80))
                                          :captures (and model.syntax
                                                         model.syntax.captures)
                                          :outer_inset context.outer_inset})]
    (when (= model.source false)
      (each [_ line (ipairs lines)]
        (set line.source_start nil)
        (set line.source_end nil)
        (each [_ part (ipairs line.spans)]
          (set part.source false)
          (set part.source_start nil)
          (set part.source_end nil))))
    {:lines (misa.layout.wrap-spans lines (math.max 1 (or context.columns 80)))}))

{: code : fields : text}
