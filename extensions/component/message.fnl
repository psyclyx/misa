;; Default transcript chrome. Markdown parsing and terminal flow are delegated to

;; the reusable markdown and component.markdown services.

(fn span [text style link] {: link : style : text})

(fn composed [base modifier]
  (if (not modifier) base (not= (type base) :table) [base modifier]
      (let [result []]
        (each [_ token (ipairs base)] (table.insert result token))
        (table.insert result modifier)
        result)))

(fn metadata [parts fact context prefix]
  (table.insert parts (span (or prefix "  ") :dim))
  (each [_ value (ipairs (misa.render_value fact context))]
    (local rendered (misa.snapshot value))
    (set rendered.style (or rendered.style :dim))
    (table.insert parts rendered)))

(fn title [model label style context]
  (let [parts [(span label (composed (or style :plain) :bold))]]
    (when (not= model.started_wall_ms nil)
      (metadata parts {:type :timestamp :value model.started_wall_ms} context))
    (when model.streaming
      (tset parts (+ (length parts) 1) (span "  streaming" :pending)))
    (when model.interrupted
      (tset parts (+ (length parts) 1) (span "  interrupted" :error)))
    (when (= (type model.tokens_per_second) :number)
      (tset parts (+ (length parts) 1)
            (span (.. "  " (string.format "%.1f" model.tokens_per_second)
                      " tok/s") :dim)))
    (when model.cost
      (metadata parts model.cost context
                (when (or model.cost.pending (and model.cost.unknown (= model.cost.amount 0)))
                  "  cost ")))
    {:spans parts}))

(fn rail [model]
  (assert model.rail "message model requires a semantic rail token"))

(fn markdown-lines [model style columns prefix previous]
  (local syntax model.syntax)
  (local projection
         (misa.markdown_view.project model.text
                            {:base style
                             :document (and syntax syntax.document)
                             :captures (and syntax syntax.captures)
                             :columns (math.max 1
                                                (- columns
                                                   (misa.layout.width prefix)))}
                                    (and previous previous.projection)))
  (local entry
         (if (and previous (= projection previous.projection)
                  (= prefix previous.prefix) (= (rail model) previous.rail)
                  (= columns previous.columns))
             previous
             {: projection : prefix : columns :rail (rail model)
              :wrapped (misa.layout.wrap_spans projection.lines columns
                                               [{:style (rail model) :text prefix}])}))
  (values entry.wrapped entry))

(fn body-lines [model context style columns prefix previous]
  (if (= context.markdown false)
      (misa.layout.wrap_spans (misa.markdown_view.plain model.text style)
                              columns [{:style (rail model) :text prefix}])
      (markdown-lines model style columns prefix previous)))

(fn interactive-message [model context style label previous]
  (local columns (math.max 1 (or (tonumber context.columns) 80)))
  (local prefix (misa.layout.clip "┃ " (math.max 0 (- columns 2))))
  ;; Titles and surfaces belong to this message wrapper; body spans stay cached.
  (local rendered [])
  (local (lines cache) (body-lines model context style columns prefix previous))
  (each [_ line (ipairs lines)]
    (local wrapped {})
    (each [key value (pairs line)] (tset wrapped key value))
    (table.insert rendered wrapped))
  (when label
    (local titles (misa.layout.wrap_spans [(title model label style context)] columns))
    (for [index (length titles) 1 (- 1)]
      (table.insert rendered 1 (. titles index))))
  ;; Title lines are newly allocated too, so assigning their surface is local.
  (each [_ line (ipairs rendered)]
    (set line.surface (.. :surface. (: (rail model) :gsub "^rail%." ""))))
  (values rendered cache))

(fn message [model context style label previous]
  (if context.interactive
      (interactive-message model context style label previous)
      (misa.markdown_view.plain model.text style)))

(fn titled [model label text style context]
  {:lines [(title model label style context)
           {:spans [(span "┃ " (rail model)) (span text style)]}]
   :surface (.. :surface. (: (rail model) :gsub "^rail%." ""))})

{:setup (fn []
          (local setup-fx [])
          (assert misa.layout "component.message requires layout")
          (assert misa.render_value "component.message requires values")
          (assert (and misa.markdown_view misa.markdown)
                  "component.message requires markdown and component.markdown")
          (fn reg [role render]
            (table.insert setup-fx
                          {:type :register/component
                           :id (.. :default. role)
                           :value {: render}}))

          (local roles
                 [{:id :user :interactive_only true :label :You :style :user}
                  {:id :assistant :label :Assistant :style :assistant}
                  {:id :thinking :label :Thinking :style :thinking}])
          (each [_ role (ipairs roles)]
            (reg (.. :transcript. role.id)
                 (fn [model context previous]
                   (local (lines cache) (when (or (not role.interactive_only) context.interactive)
                                         (message model context role.style role.label previous)))
                   (values {:lines (or lines [])} cache))))
          (reg :transcript.thinking_collapsed
               (fn [model context]
                 (titled model :Thinking (tostring (or model.summary :summary))
                         :thinking context)))
          (reg :transcript.harness
               (fn [model context previous]
                 (local (lines cache) (message model context (if (= model.level :error) :error :plain)
                                              nil previous))
                 (values {: lines} cache)))
          {:fx setup-fx})}
