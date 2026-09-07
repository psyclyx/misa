;; Default transcript chrome. Markdown parsing and terminal flow are delegated to

;; the reusable markdown and component.markdown services.

(fn span [text style link] {: link : style : text})

(fn composed [base modifier]
  (if (not modifier) base (not= (type base) :table) [base modifier]
      (let [result []]
        (each [_ token (ipairs base)] (table.insert result token))
        (table.insert result modifier)
        result)))

(fn title [model label style]
  (let [parts [(span label (composed (or style :plain) :bold))]]
    (when model.timestamp
      (tset parts (+ (length parts) 1)
            (span (.. "  " (tostring model.timestamp)) :dim)))
    (when model.streaming
      (tset parts (+ (length parts) 1) (span "  streaming" :pending)))
    (when model.interrupted
      (tset parts (+ (length parts) 1) (span "  interrupted" :error)))
    (when (= (type model.tokens_per_second) :number)
      (tset parts (+ (length parts) 1)
            (span (.. "  " (string.format "%.1f" model.tokens_per_second)
                      " tok/s") :dim)))
    (when model.cost
      (tset parts (+ (length parts) 1) (span (.. "  " model.cost) :dim)))
    {:spans parts}))

(fn rail [model]
  (assert model.rail "message model requires a semantic rail token"))

(var documents {})

(fn markdown-lines [model style columns prefix]
  (local syntax (and model.syntax
                     (if (= (type model.syntax) :function) (model.syntax)
                         model.syntax)))
  (local owner (tostring (or model.response_id "")))
  (local key (and model.id (.. (length owner) ":" owner (tostring model.id))))
  (var entry (and key (. documents key)))
  (when (not entry)
    (set entry {:view (misa.markdown_view.new_document)})
    (when key (tset documents key entry)))
  (local lines
         (entry.view:render model.text
                            {:base style
                             :document (and syntax syntax.document)
                             :captures (and syntax syntax.captures)
                             :columns (math.max 1
                                                (- columns
                                                   (misa.layout.width prefix)))}))
  (when (or (or (or (not= lines entry.lines) (not= prefix entry.prefix))
                (not= (rail model) entry.rail))
            (not= columns entry.columns))
    (set entry.wrapped
         (misa.layout.wrap_spans lines columns
                                 [{:style (rail model) :text prefix}]))
    (set (entry.lines entry.prefix entry.rail entry.columns)
         (values lines prefix (rail model) columns)))
  entry.wrapped)

(fn body-lines [model context style columns prefix]
  (if (= context.markdown false)
      (misa.layout.wrap_spans (misa.markdown_view.plain model.text style)
                              columns [{:style (rail model) :text prefix}])
      (markdown-lines model style columns prefix)))

(fn interactive-message [model context style label]
  (local columns (math.max 1 (or (tonumber context.columns) 80)))
  (local prefix (misa.layout.clip "┃ " (math.max 0 (- columns 2))))
  ;; Titles and surfaces belong to this message wrapper; body spans stay cached.
  (local rendered [])
  (each [_ line (ipairs (body-lines model context style columns prefix))]
    (local wrapped {})
    (each [key value (pairs line)] (tset wrapped key value))
    (table.insert rendered wrapped))
  (when label
    (local titles (misa.layout.wrap_spans [(title model label style)] columns))
    (for [index (length titles) 1 (- 1)]
      (table.insert rendered 1 (. titles index))))
  ;; Title lines are newly allocated too, so assigning their surface is local.
  (each [_ line (ipairs rendered)]
    (set line.surface (.. :surface. (: (rail model) :gsub "^rail%." ""))))
  rendered)

(fn message [model context style label]
  (if context.interactive
      (interactive-message model context style label)
      (misa.markdown_view.plain model.text style)))

(fn titled [model label text style]
  {:lines [(title model label style)
           {:spans [(span "┃ " (rail model)) (span text style)]}]
   :surface (.. :surface. (: (rail model) :gsub "^rail%." ""))})

{:setup (fn []
          (local setup-fx [])
          (assert misa.layout "component.message requires layout")
          (assert (and misa.markdown_view misa.markdown)
                  "component.message requires markdown and component.markdown")
          ;; One derived document per transcript block, released with its transcript.
          ;; No parser state enters the transactional database or persisted history.
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [] (set documents {}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/reset
                         :handler (fn [] (set documents {}))})

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
                 (fn [model context]
                   {:lines (or (and (or (not role.interactive_only)
                                        context.interactive)
                                    (message model context role.style
                                             role.label))
                               {})})))
          (reg :transcript.thinking_collapsed
               (fn [model]
                 (titled model :Thinking (tostring (or model.summary :summary))
                         :thinking)))
          (reg :transcript.harness
               (fn [model context]
                 {:lines (message model context
                                  (or (and (= model.level :error) :error)
                                      :plain)
                                  nil)}))
          {:fx setup-fx})}
