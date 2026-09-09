;; Tool lifecycle chrome composes ordinary content views. Bindings to actual
;; tools live in tool.presentations; this wrapper does not inspect definitions.
(fn span [text style] {: text : style :source false})
(local status-styles {:cancelled :tool.cancelled
                      :error :tool.error
                      :success :tool.success})

(local status-markers {:cancelled "⊘ cancelled"
                       :pending "… pending"
                       :running "… running"})

(fn title [model subject context]
  (let [status (or model.status (if model.is_error :error :success))
        style (or (. status-styles status) :tool.pending)
        spans [(span (.. "◇ " (or model.name "Tool result")) [style :bold])]]
    (when subject
      (table.insert spans (span "  " :dim))
      (each [_ value (ipairs (misa.values.render subject context))]
        (table.insert spans (misa.patch value {:style :tool :source false}))))
    (when (. status-markers status)
      (table.insert spans (span (.. "  " (. status-markers status)) style)))
    (when (not= model.elapsed_ms nil)
      (table.insert spans (span "  " :dim))
      (each [_ value (ipairs (misa.values.render {:type :duration
                                                  :value model.elapsed_ms}
                                                 context))]
        (table.insert spans (misa.patch value {:style :dim :source false}))))
    {: spans}))

(fn render [model context]
  "Render the supplied semantic model within the available dimensions."
  (assert context.render_child "component.tool requires component composition")
  (let [columns (math.max 1 (or context.columns 80))
        prefix (misa.layout.clip "┃ " (math.max 0 (- columns 2)))
        content-columns (math.max 1 (- columns (misa.layout.width prefix)))
        child-context {:columns content-columns
                       :interactive context.interactive
                       :outer_inset (misa.layout.width prefix)}
        fallback (= model.kind :tool_result)
        views (misa.tools.presentation model)
        rail (or model.rail (if model.is_error :rail.error :rail.tool))
        surface (when context.interactive
                  (if model.is_error :surface.error :surface.tool))
        lines (misa.layout.wrap-spans [(misa.patch (title model views.subject
                                                          context)
                                                   {: surface})]
                                      columns [(span prefix rail)])]
    (var has-content false)

    (fn content [descriptors part collapsed]
      (let [rows []]
        (var limit 3)
        (var tail false)
        (each [_ descriptor (ipairs descriptors)]
          (set limit (math.max limit (or descriptor.model.preview_limit 3)))
          (set tail (or tail descriptor.model.preview_tail))
          (let [rendered (context.render_child descriptor.role descriptor.model
                                               child-context)]
            (each [index line (ipairs rendered.lines)]
              (let [row (misa.patch line
                                    {:source_part part
                                     : surface
                                     :block_start (= index 1)})]
                (when (and model.selection_source
                           (not= model.selection_source part))
                  (set row.source_start nil)
                  (set row.source_end nil)
                  (set row.spans
                       (icollect [_ value (ipairs row.spans)]
                         (misa.patch value
                                     {:source false
                                      :source_start misa.delete
                                      :source_end misa.delete}))))
                (table.insert rows row)))))
        (let [visible (if collapsed
                          (. (context.render_child :content.truncated
                                                   {:lines rows : limit : tail}
                                                   child-context)
                             :lines)
                          rows)
              spaced []]
          (each [index line (ipairs visible)]
            (when (or (= index 1) line.block_start)
              (table.insert spaced {:spans [] : surface}))
            (table.insert spaced line))
          (when (> (length visible) 0) (set has-content true))
          (each [_ line (ipairs (misa.layout.wrap-spans spaced columns
                                                        [(span prefix rail)]))]
            (table.insert lines (misa.patch line {: surface})))
          (length visible))))

    (when (not fallback)
      (let [args-selected (= model.selection_source :args)]
        (content (if args-selected
                     [{:role :content.text
                       :model {:text model.selection_text :style :tool}}]
                     views.arguments) :args model.collapsed)))
    (let [result-text (if (= model.selection_source :result)
                          model.selection_text
                          (not= model.result nil)
                          model.result
                          fallback
                          model.text
                          nil)]
      (when (not= result-text nil)
        (let [collapsed model.collapsed
              summary (and collapsed (not views.prefer_result)
                           (not (and views.result
                                     views.result.model.preview_tail))
                           (if model.is_error
                               (: (or (result-text:match "[^\r\n]+") "") :gsub
                                  "^.-:%d+:%s*" "")
                               model.summary))
              view (if (and summary views.result
                            (= views.result.role :content.code))
                       {:role :content.code
                        :model (misa.patch views.result.model
                                           {:text summary
                                            :source false
                                            :missing_newline misa.delete})}
                       (or summary (= model.selection_source :result))
                       {:role :content.text
                        :model {:text (or summary result-text)
                                :style :tool
                                :source (not summary)}}
                       views.result)]
          (when view (content [view] :result collapsed))))
      (when has-content
        (table.insert lines {:spans [(span prefix rail)] : surface}))
      {: lines})))

{: render}
