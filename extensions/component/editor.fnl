;; Default editor visuals.

(fn span [text style] {: style : text})

(local markers {:insert "│ " :normal "◆ " :visual "◇ "})

{:setup (fn []
          (local setup-fx [])
          (assert (and misa.layout misa.layout.wrap_input)
                  "component.editor requires wrapped input layout")
          (table.insert setup-fx
                        {:type :register/component
                         :id :default.editor.input
                         :value {:render (fn [model context]
                                           (local mode (or model.mode :insert))
                                           (local marker (or (. markers mode) markers.insert))
                                           (local prompt-style
                                                  (or (and (= mode :insert)
                                                           :accent)
                                                      (.. :editor. mode)))
                                           (local rendered
                                                  (misa.layout.wrap_input model.text
                                                                          (or context.columns
                                                                              80)
                                                                          (or model.cursor
                                                                              (length (or model.text
                                                                                          "")))
                                                                          marker
                                                                          :user
                                                                          prompt-style))
                                           (var source-at 0)
                                           (each [index line (ipairs rendered.lines)]
                                             (local previous
                                                    (. line.spans 1 :text))
                                             (local prefix
                                                  (misa.layout.clip (if (= index 1) marker markers.insert)
                                                                    (misa.layout.width previous)))
                                             (tset (. line.spans 1) :text
                                                   prefix)
                                             (when (and rendered.cursor
                                                        (= rendered.cursor.row
                                                           index))
                                               (set rendered.cursor.byte
                                                    (- (+ rendered.cursor.byte
                                                          (length prefix))
                                                       (length previous))))
                                             (local text (. line.spans 2 :text))
                                             (when (and (and (= mode :visual)
                                                             model.selection_start)
                                                        model.selection_end)
                                               (local first
                                                      (math.max 0
                                                                (math.min (length text)
                                                                          (- model.selection_start
                                                                             source-at))))
                                               (local last
                                                      (math.max first
                                                                (math.min (length text)
                                                                          (- model.selection_end
                                                                             source-at))))
                                               (tset line.spans 2
                                                     {:style :user
                                                      :text (text:sub 1 first)})
                                               (tset line.spans 3
                                                     {:style [:user :selection]
                                                      :text (text:sub (+ first
                                                                         1)
                                                                      last)})
                                               (tset line.spans 4
                                                     {:style :user
                                                      :text (text:sub (+ last 1))}))
                                             (set source-at
                                                  (+ source-at (length text)))
                                             (when (= (: (or model.text "")
                                                         :sub (+ source-at 1)
                                                         (+ source-at 1))
                                                      "\n")
                                               (set source-at (+ source-at 1))))
                                           rendered)}})
          (table.insert setup-fx
                        {:type :register/component
                         :id :default.editor.completions
                         :value {:render (fn [model]
                                           (if model.lines
                                               (do
                                                 (local lines {})
                                                 (each [_ line (ipairs model.lines)]
                                                   (tset lines
                                                         (+ (length lines) 1)
                                                         line))
                                                 (when model.overflow
                                                   (tset lines
                                                         (+ (length lines) 1)
                                                         {:spans [(span model.overflow
                                                                        :choice.hint)]}))
                                                 {: lines})
                                               (do
                                                 (local rendered {})
                                                 (each [_ row (ipairs (or model.rows
                                                                          {}))]
                                                   (local style
                                                          (or (and row.selected
                                                                   :choice.row.selected)
                                                              (or (and row.active
                                                                       :choice.row.active)
                                                                  :choice.row)))
                                                   (local spans
                                                          [(span (.. (or row.marker
                                                                         " ")
                                                                     " ")
                                                                 style)])
                                                   (when row.hotkey
                                                     (each [_ key-span (ipairs (or (and misa.render_keybinding
                                                                                        (misa.render_keybinding row.hotkey))
                                                                                   [(span row.hotkey
                                                                                          :keybinding)]))]
                                                       (tset spans
                                                             (+ (length spans)
                                                                1)
                                                             key-span))
                                                     (tset spans
                                                           (+ (length spans) 1)
                                                           (span " " :plain)))
                                                   (tset spans
                                                         (+ (length spans) 1)
                                                         (span row.label style))
                                                   (tset spans
                                                         (+ (length spans) 1)
                                                         (span (or (and (and row.description
                                                                             (not= row.description
                                                                                   ""))
                                                                        (.. "  "
                                                                            row.description))
                                                                   "")
                                                               :choice.hint))
                                                   (tset rendered
                                                         (+ (length rendered) 1)
                                                         {: spans}))
                                                 (when model.overflow
                                                   (tset rendered
                                                         (+ (length rendered) 1)
                                                         {:spans [(span model.overflow
                                                                        :choice.hint)]}))
                                                 {:lines rendered})))}})
          {:fx setup-fx})}
