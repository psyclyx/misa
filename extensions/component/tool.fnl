;; Tool lifecycle chrome composes ordinary content views. Bindings to actual
;; tools live in tool_presentations; this wrapper does not inspect definitions.
(fn span [text style] {: text : style :source false})
(local status-styles {:cancelled :tool.cancelled :error :tool.error :success :tool.success})
(local status-markers {:cancelled "⊘ cancelled"
                       :pending "… pending" :running "… running"})
(fn title [model subject context]
  (local status (or model.status (if model.is_error :error :success)))
  (local style (or (. status-styles status) :tool.pending))
  (local spans [(span (.. "◇ " (or model.name "Tool result")) [style :bold])])
  (when subject
    (table.insert spans (span "  " :dim))
    (each [_ value (ipairs (misa.render_value subject context))]
      (table.insert spans (misa.patch value {:style :tool :source false}))))
  (when (. status-markers status)
    (table.insert spans (span (.. "  " (. status-markers status)) style)))
  (when (not= model.elapsed_ms nil)
    (table.insert spans (span "  " :dim))
    (each [_ value (ipairs (misa.render_value {:type :duration :value model.elapsed_ms} context))]
      (table.insert spans (misa.patch value {:style :dim :source false}))))
  {: spans})

(fn render [model context]
  (assert context.render_child "component.tool requires component composition")
  (local columns (math.max 1 (or context.columns 80)))
  (local prefix (misa.layout.clip "┃ " (math.max 0 (- columns 2))))
  (local content-columns (math.max 1 (- columns (misa.layout.width prefix))))
  (local child-context {:columns content-columns :interactive context.interactive
                        :outer_inset (misa.layout.width prefix)})
  (local fallback (= model.kind :tool_result))
  (local views (misa.tool_presentation model))
  (local rail (or model.rail (if model.is_error :rail.error :rail.tool)))
  (local surface (when context.interactive (if model.is_error :surface.error :surface.tool)))
  (local lines (misa.layout.wrap_spans [(misa.patch (title model views.subject context) {: surface})]
                                       columns [(span prefix rail)]))
  (var has-content false)
  (fn content [descriptors part collapsed]
    (local rows [])
    (var limit 3)
    (var tail false)
    (each [_ descriptor (ipairs descriptors)]
      (set limit (math.max limit (or descriptor.model.preview_limit 3)))
      (set tail (or tail descriptor.model.preview_tail))
      (local rendered (context.render_child descriptor.role descriptor.model child-context))
      (each [index line (ipairs rendered.lines)]
        (local row (misa.patch line {:source_part part : surface :block_start (= index 1)}))
        (when (and model.selection_source (not= model.selection_source part))
          (set row.source_start nil)
          (set row.source_end nil)
          (set row.spans (icollect [_ value (ipairs row.spans)]
                           (misa.patch value {:source false :source_start misa.delete :source_end misa.delete}))))
        (table.insert rows row)))
    (local visible (if collapsed
                      (. (context.render_child :content.truncated {:lines rows : limit : tail} child-context) :lines)
                      rows))
    (local spaced [])
    (each [index line (ipairs visible)]
      (when (or (= index 1) line.block_start)
        (table.insert spaced {:spans [] : surface}))
      (table.insert spaced line))
    (when (> (length visible) 0) (set has-content true))
    (each [_ line (ipairs (misa.layout.wrap_spans spaced columns [(span prefix rail)]))]
      (table.insert lines (misa.patch line {: surface})))
    (length visible))
  (when (not fallback)
    (local args-selected (= model.selection_source :args))
    (content (if args-selected
                 [{:role :content.text :model {:text model.selection_text :style :tool}}]
                 views.arguments)
             :args model.collapsed))
  (local result-text (if (= model.selection_source :result) model.selection_text
                        (not= model.result nil) model.result fallback model.text nil))
  (when (not= result-text nil)
    (local collapsed model.collapsed)
    (local summary (and collapsed (not views.prefer_result)
                        (not (and views.result views.result.model.preview_tail)) (if model.is_error
                                     (: (or (result-text:match "[^\r\n]+") "") :gsub "^.-:%d+:%s*" "") model.summary)))
    (local view (if (and summary views.result (= views.result.role :content.code))
                   {:role :content.code :model (misa.patch views.result.model {:text summary :source false :missing_newline misa.delete})}
                   (or summary (= model.selection_source :result))
                   {:role :content.text :model {:text (or summary result-text) :style :tool :source (not summary)}}
                   views.result))
    (when view (content [view] :result collapsed)))
  (when has-content (table.insert lines {:spans [(span prefix rail)] : surface}))
  {: lines})

{:setup (fn []
          (assert (and misa.layout misa.render_value misa.tool_presentation)
                  "component.tool requires layout, values, and tool_presentations")
          {:fx [{:type :register/component :id :default.transcript.tool_call :value {: render :compose true}}
                {:type :register/component :id :default.transcript.tool_result :value {: render :compose true}}]})}
