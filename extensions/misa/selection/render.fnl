(local definitions (require :misa.definitions))

;; Selection status belongs beside the input; the transcript remains the view.

(fn render-selection [model context]
  (let [path (: (table.concat (or model.path {}) " / ") :gsub "[\r\n]" " ")
        lines [{:spans [{:style :label
                         :text (misa.layout.clip (.. (or (and model.copied
                                                              "Copied · ")
                                                         (and model.visual
                                                              "Visual · ")
                                                         "Select · ")
                                                     path)
                                                 (or context.columns 80))}]}
               {:spans (misa.keybindings.reference model.hints)}]]
    (while (> (length lines)
              (math.max 0 (or context.available_lines (length lines))))
      (table.remove lines))
    {:dock :input :input_disabled true : lines}))

(fn build []
  "Declare selection rendering."
  (definitions.build :component.selection
    [{:catalog :components
      :id :default.selection
      :value {:render render-selection}}]
    {}))

{: build}
