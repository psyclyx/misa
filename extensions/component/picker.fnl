;; Pure renderer for the complete geometry produced by choice_layout.

(fn span [text style] {: style : text})

(fn append [target source]
  (each [_ value (ipairs (or source {}))]
    (tset target (+ (length target) 1) value))
  nil)

(fn fit-line [line width]
  (var (spans remaining) (values {} width))
  (each [_ part (ipairs (or (and line line.spans) {}))]
    (when (> remaining 0)
      (local (text _ used) (misa.layout.take (or part.text "") remaining))
      (local fitted (misa.snapshot part))
      (set fitted.text text)
      (table.insert spans fitted)
      (set remaining (- remaining used))))
  (when (> remaining 0)
    (tset spans (+ (length spans) 1) (span (string.rep " " remaining) :plain)))
  spans)

{:setup (fn []
          (local setup-fx [])
          (assert misa.layout "component.picker requires layout")
          (table.insert setup-fx
                        {:type :register/component
                         :id :default.picker
                         :value {:render (fn [model]
                                           (local pad
                                                  (string.rep " "
                                                              (or model.x 0)))
                                           (local result {})
                                           (when (not model.input.hidden)
                                             (tset result (+ (length result) 1)
                                                   {:spans [(span pad :plain)
                                                            (span (.. model.input.title
                                                                      ": ")
                                                                  :choice.prompt)
                                                            (span model.input.text
                                                                  :choice.query)]}))
                                           (for [index 1 model.preview.height]
                                             (local spans [(span pad :plain)])
                                             (append spans (or (. model.preview.lines index :spans) []))
                                             (table.insert result {: spans}))
                                           (local rendered {})
                                           (each [bank column (ipairs model.columns)]
                                             (local lines
                                                    [{:spans [(span column.title
                                                                    (or (and column.active
                                                                             :choice.view.active)
                                                                        :choice.view))]}])
                                             (when (and (= (length column.rows)
                                                           0)
                                                        (not column.overflow))
                                               (tset lines (+ (length lines) 1)
                                                     {:spans [(span "  no matches"
                                                                    :choice.empty)]}))
                                             (append lines column.lines)
                                             (when column.overflow
                                               (tset lines (+ (length lines) 1)
                                                     {:spans [(span column.overflow
                                                                    :choice.hint)]}))
                                             (tset rendered bank lines))
                                           (for [line-index 1 model.panel_height]
                                             (local spans [(span pad :plain)])
                                             (var any false)
                                             (each [bank column (ipairs model.columns)]
                                               (local line
                                                      (. rendered bank
                                                         line-index))
                                               (when line (set any true))
                                               (append spans
                                                       (fit-line line
                                                                 column.width))
                                               (when (< bank
                                                        (length model.columns))
                                                 (tset spans
                                                       (+ (length spans) 1)
                                                       (span "  " :plain))))
                                             (when (not any) (lua :break))
                                             (tset result (+ (length result) 1)
                                                   {: spans}))
                                           (when (> (or model.hint_height
                                                        (or (and (> (length (or model.hints
                                                                                {}))
                                                                    0)
                                                                 1)
                                                            0))
                                                    0)
                                             (local hint-spans
                                                    [(span pad :plain)])
                                             (append hint-spans
                                                     (or (and misa.render_keybinding_reference
                                                              (misa.render_keybinding_reference model.hints))
                                                         {}))
                                             (tset result (+ (length result) 1)
                                                   {:spans hint-spans}))
                                           (while (> (length result)
                                                     model.height)
                                             (table.remove result))
                                           {:cursor (or (and (not model.input.hidden)
                                                             {:byte (+ (length pad)
                                                                       (length model.input.title)
                                                                       2
                                                                       model.input.cursor)
                                                              :row 1})
                                                        nil)
                                            :exclusive false
                                            :height model.height
                                            :lines result
                                            :overlay true})}})
          {:fx setup-fx})}
