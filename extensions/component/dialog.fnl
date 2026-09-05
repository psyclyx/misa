;; Generic popup content. Root composition retains the transcript behind it.

(fn span [text style link] {: link : style : text})

(local fields [{:key :message :omit_empty true :style :dialog.message}
               {:key :url :label "URL  " :link true :style :link}
               {:key :code :label "Code " :style :dialog.code}
               {:key :progress :style :dialog.progress}
               {:key :input_error :style :dialog.message}])

{:setup (fn []
          (local setup-fx [])
          (assert misa.layout "component.dialog requires layout")
          (table.insert setup-fx
                        {:type :register/component
                         :id :default.dialog
                         :value {:render (fn [model context]
                                           (local width
                                                  (math.max 1
                                                            (- (or context.columns
                                                                   80)
                                                               2)))
                                           (local lines {})

                                           (fn add [spans]
                                             (each [_ line (ipairs (misa.layout.wrap_spans [{: spans}]
                                                                                           width))]
                                               (table.insert line.spans 1
                                                             (span "│ "
                                                                   :dialog.label))
                                               (tset lines (+ (length lines) 1)
                                                     line)))

                                           (tset lines (+ (length lines) 1)
                                                 {:spans [(span "┌─ "
                                                                :dialog.label)
                                                          (span (or (and (not= model.title
                                                                               "")
                                                                         model.title)
                                                                    :Interaction)
                                                                :dialog.title)]})
                                           (each [_ field (ipairs fields)]
                                             (local value (. model field.key))
                                             (when (and value
                                                        (or (not field.omit_empty)
                                                            (not= value "")))
                                               (local spans {})
                                               (when field.label
                                                 (tset spans
                                                       (+ (length spans) 1)
                                                       (span field.label
                                                             :dialog.label)))
                                               (tset spans (+ (length spans) 1)
                                                     (span (tostring value)
                                                           field.style
                                                           (or (and field.link
                                                                    value)
                                                               nil)))
                                               (add spans)))
                                           (var (input-lines cursor) nil)
                                           (when model.input_enabled
                                             (local text
                                                    (or (and model.protected
                                                             (string.rep "•"
                                                                         (math.min (or model.input_length
                                                                                       0)
                                                                                   (math.max 1
                                                                                             (- width
                                                                                                2)))))
                                                        (or model.input "")))
                                             (set input-lines
                                                  (misa.layout.wrap_input text
                                                                          (or context.columns
                                                                              80)
                                                                          (length text)
                                                                          "│ "
                                                                          :dialog.input
                                                                          :dialog.label)))
                                           (local hints {})
                                           (each [_ hint (ipairs (or model.hints
                                                                     {}))]
                                             (tset hints (+ (length hints) 1)
                                                   {:text (tostring hint)}))
                                           (each [index action (ipairs (or model.actions
                                                                           {}))]
                                             (tset hints (+ (length hints) 1)
                                                   {:text (or (and (= index
                                                                      (or model.selected_action
                                                                          1))
                                                                   (.. "["
                                                                       action.label
                                                                       "]"))
                                                              action.label)}))
                                           (when (> (length (or model.actions
                                                                {}))
                                                    1)
                                             (tset hints (+ (length hints) 1)
                                                   {:key :tab
                                                    :label "next action"}))
                                           (when model.cancellable
                                             (tset hints (+ (length hints) 1)
                                                   {:key :escape
                                                    :label :cancel}))
                                           (local footer
                                                  [(span "└─ "
                                                         :dialog.label)])
                                           (each [index hint (ipairs hints)]
                                             (when (> index 1)
                                               (tset footer
                                                     (+ (length footer) 1)
                                                     (span "    " :dialog.hint)))
                                             (if (and hint.key
                                                      misa.render_keybinding)
                                                 (do
                                                   (each [_ key-span (ipairs (misa.render_keybinding hint.key))]
                                                     (tset footer
                                                           (+ (length footer) 1)
                                                           key-span))
                                                   (tset footer
                                                         (+ (length footer) 1)
                                                         (span (.. " "
                                                                   hint.label)
                                                               :dialog.hint)))
                                                 (tset footer
                                                       (+ (length footer) 1)
                                                       (span hint.text
                                                             :dialog.hint))))
                                           (local room
                                                  (math.max 0
                                                            (or context.available_lines
                                                                24)))
                                           (local input-room
                                                  (or (and input-lines
                                                           (math.min (length input-lines.lines)
                                                                     (math.max 1
                                                                               (- room
                                                                                  2))))
                                                      0))
                                           (local body-room
                                                  (math.max 0
                                                            (- (- room
                                                                  input-room)
                                                               1)))
                                           (when (> (length lines) body-room)
                                             (while (> (length lines) body-room)
                                               (table.remove lines))
                                             (when (> body-room 1)
                                               (tset lines body-room
                                                     {:spans [(span "│ … more content"
                                                                    :dialog.hint)]})))
                                           (when (and input-lines (> room 1))
                                             (local first
                                                    (math.max 1
                                                              (+ (- input-lines.cursor.row
                                                                    input-room)
                                                                 1)))
                                             (local offset (length lines))
                                             (for [row first (math.min (length input-lines.lines)
                                                                       (- (+ first
                                                                             input-room)
                                                                          1))]
                                               (tset lines (+ (length lines) 1)
                                                     (. input-lines.lines row)))
                                             (set cursor
                                                  {:byte input-lines.cursor.byte
                                                   :row (+ (- (+ offset
                                                                 input-lines.cursor.row)
                                                              first)
                                                           1)}))
                                           (when (> room 0)
                                             (tset lines (+ (length lines) 1)
                                                   {:spans footer}))
                                           {: cursor
                                            : lines
                                            :overlay true
                                            :surface :surface.dialog})}})
          {:fx setup-fx})}
