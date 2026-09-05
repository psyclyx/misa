;; Reusable tool-section visuals. A single section presents the call metadata,

;; arguments, lifecycle indicator, and eventual result.

(fn span [text style] {: style : text})

(fn composed [base modifier]
  (if (not modifier) base
      (not= (type base) :table) [base modifier]
      (let [result []]
        (each [_ token (ipairs base)] (table.insert result token))
        (table.insert result modifier)
        result)))

(local status-styles {:cancelled :tool.cancelled
                      :error :tool.error
                      :success :tool.success})

(fn status-style [status] (or (. status-styles status) :tool.pending))

(fn title [model label]
  (let [status (or model.status (or (and model.is_error :error) :success))
        parts [(span label (composed (status-style status) :bold))]]
    (when (and model.description (not= model.description ""))
      (tset parts (+ (length parts) 1) (span (.. "  " model.description) :dim)))
    (when model.timestamp
      (tset parts (+ (length parts) 1)
            (span (.. "  " (tostring model.timestamp)) :dim)))
    (tset parts (+ (length parts) 1)
          (span (.. "  " status) (status-style status)))
    {:spans parts}))

(fn bodies [text style rail prefix source]
  (let [result []
        normalized (: (: (tostring (or text "")) :gsub "\r\n" "\n") :gsub "\r" "\n")]
    (var offset 0)
    (each [line (: (.. normalized "\n") :gmatch "(.-)\n")]
      (local content (span line style))
      (set content.source (not= source false))
      (tset result (+ (length result) 1)
            {:source_end (+ offset (length line))
             :source_start offset
             :spans [(span "┃ " rail) (span (or prefix "") style) content]})
      (set offset (+ offset (length line) 1)))
    result))

(fn append [target source]
  (each [_ line (ipairs source)]
    (tset target (+ (length target) 1) line)))

{:setup (fn []
          (assert misa.layout "component.tool requires layout")

          (fn render [model context]
            (local fallback (and (= model.kind :tool_result) (not model.name)))
            (local label
                   (or (and fallback
                            (or (and model.is_error "Tool error") "Tool result"))
                       (.. "Tool · " (tostring (or model.name :tool)))))
            (local rail
                   (or model.rail
                       (or (and model.is_error :rail.error) :rail.tool)))
            (local lines [(title model label)])
            (when (not fallback)
              (append lines
                      (bodies (tostring (or model.detail :summary)) :tool rail
                              "args  " (not= model.selection_source :result))))
            (if (not= model.result nil)
                (append lines
                        (bodies (tostring (or model.result_detail model.result))
                                :tool rail "result  "
                                (not= model.selection_source :args)))
                fallback (append lines
                                (bodies (or (and model.collapsed :summary)
                                            model.text)
                                        :tool rail)))
            {:lines (misa.layout.wrap_spans lines (or context.columns 80))
             :surface (or (and context.interactive
                               (or (and model.is_error :surface.error)
                                   :surface.tool))
                          nil)})

          (misa.reg_component :default.transcript.tool_call {: render})
          (misa.reg_component :default.transcript.tool_result {: render}))}

