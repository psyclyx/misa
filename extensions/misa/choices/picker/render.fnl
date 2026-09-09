;; Pure renderer for the complete geometry produced by choices.layout.

(fn span [text style] {: style : text})

(fn append [target source]
  (each [_ value (ipairs (or source {}))]
    (tset target (+ (length target) 1) value))
  nil)

(fn fit-line [line width]
  (let [spans {}]
    (var remaining width)
    (each [_ part (ipairs (or (and line line.spans) {}))]
      (when (> remaining 0)
        (let [(text _ used) (misa.layout.take (or part.text "") remaining)
              fitted (misa.snapshot part)]
          (set fitted.text text)
          (table.insert spans fitted)
          (set remaining (- remaining used)))))
    (when (> remaining 0)
      (tset spans (+ (length spans) 1) (span (string.rep " " remaining) :plain)))
    spans))

(fn render-picker [model]
  "Render picker content."
  (let [pad (string.rep " " (or model.x 0))
        result {}]
    (when (not model.input.hidden)
      (tset result (+ (length result) 1)
            {:spans [(span pad :plain)
                     (span (.. model.input.title ": ") :choice.prompt)
                     (span model.input.text :choice.query)]}))
    (for [index 1 model.preview.height]
      (let [spans [(span pad :plain)]]
        (append spans (or (. model.preview.lines index :spans) []))
        (table.insert result {: spans})))
    (let [rendered {}]
      (each [bank column (ipairs model.columns)]
        (let [lines [{:spans [(span column.title
                                    (or (and column.active :choice.view.active)
                                        :choice.view))]}]]
          (when (and (= (length column.rows) 0) (not column.overflow))
            (tset lines (+ (length lines) 1)
                  {:spans [(span "  no matches" :choice.empty)]}))
          (append lines column.lines)
          (when column.overflow
            (tset lines (+ (length lines) 1)
                  {:spans [(span column.overflow :choice.hint)]}))
          (tset rendered bank lines)))
      (let [height (accumulate [height 0 _ lines (ipairs rendered)]
                     (math.max height (length lines)))]
        (for [line-index 1 (math.min model.panel_height height)]
          (let [spans [(span pad :plain)]]
            (each [bank column (ipairs model.columns)]
              (let [line (. rendered bank line-index)]
                (append spans (fit-line line column.width))
                (when (< bank (length model.columns))
                  (tset spans (+ (length spans) 1) (span "  " :plain)))))
            (tset result (+ (length result) 1) {: spans}))))
      (when (> (or model.hint_height (and (> (length (or model.hints {})) 0) 1)
                   0) 0)
        (let [hint-spans [(span pad :plain)]]
          (append hint-spans (misa.keybindings.reference model.hints))
          (tset result (+ (length result) 1) {:spans hint-spans})))
      (while (> (length result) model.height)
        (table.remove result))
      {:cursor (or (and (not model.input.hidden)
                        {:byte (+ (length pad) (length model.input.title) 2
                                  model.input.cursor)
                         :row 1}) nil)
       :exclusive false
       :height model.height
       :lines result
       :overlay true})))

{:render-picker render-picker}
