;; Transcript chrome delegates document parsing and terminal flow to Markdown.

(fn rail [model]
  (assert model.rail "message model requires a semantic rail token"))

(fn markdown-lines [model style columns prefix previous]
  (let [syntax model.syntax
        projection (misa.markdown.view.project model.text
                                               {:base style
                                                :document (and syntax
                                                               syntax.document)
                                                :captures (and syntax
                                                               syntax.captures)
                                                :outer_inset (misa.layout.width prefix)
                                                :columns (math.max 1
                                                                   (- columns
                                                                      (misa.layout.width prefix)))}
                                               (and previous
                                                    previous.projection))
        entry (if (and previous (= projection previous.projection)
                       (= prefix previous.prefix) (= (rail model) previous.rail)
                       (= columns previous.columns))
                  previous
                  {: projection
                   : prefix
                   : columns
                   :rail (rail model)
                   :wrapped (misa.layout.wrap-spans projection.lines columns
                                                    [{:style (rail model)
                                                      :text prefix}])})]
    (values entry.wrapped entry)))

(fn body-lines [model context style columns prefix previous]
  (if (= context.markdown false)
      (misa.layout.wrap-spans (misa.markdown.view.plain model.text style)
                              columns [{:style (rail model) :text prefix}])
      (markdown-lines model style columns prefix previous)))

(fn interactive-message [model context style previous limit]
  (let [columns (math.max 1 (or (tonumber context.columns) 80))
        prefix (misa.layout.clip "┃ " (math.max 0 (- columns 2)))
        ;; The rail and its surface are one continuous block, including its chrome.
        rendered []
        (lines cache) (body-lines model context style columns prefix previous)
        visible (if limit
                    (. (context.render_child :content.truncated
                                             {: lines
                                              : limit
                                              :notice_prefix [{:text prefix
                                                               :style (rail model)
                                                               :source false}]}
                                             context)
                       :lines)
                    lines)]
    (each [_ line (ipairs visible)]
      (let [wrapped {}]
        (each [key value (pairs line)] (tset wrapped key value))
        (set wrapped.surface (.. :surface. (: (rail model) :gsub "^rail%." "")))
        (table.insert rendered wrapped)))
    (values rendered cache)))

(fn message [model context style previous]
  (if context.interactive
      (interactive-message model context style previous)
      (misa.markdown.view.plain model.text style)))

(fn render-message [style interactive-only? model context previous]
  "Render a message with its semantic style and visibility policy."
  (let [(lines cache) (when (or (not interactive-only?) context.interactive)
                        (message model context style previous))]
    (values {:lines (or lines [])} cache)))

(fn collapsed [model context previous]
  "Render a bounded preview of a thinking message."
  (let [(lines cache) (interactive-message model context :thinking previous 3)]
    (values {: lines} cache)))

(fn harness [model context previous]
  "Render a harness message using its severity."
  (let [(lines cache) (message model context
                               (if (= model.level :error) :error :plain)
                               previous)]
    (values {: lines} cache)))

{: collapsed : harness : render-message}
