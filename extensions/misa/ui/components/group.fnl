(local definitions (require :misa.definitions))

(fn boundary [model context]
  (let [columns (math.max 1 (or context.columns 80))
        parts [{:text "──" :style :dim}]]
    (fn append [spans]
      (table.insert parts {:text "  " :style :dim})
      (each [_ part (ipairs spans)]
        (table.insert parts (misa.patch part {:style :dim :source false}))))

    (when model.label
      (append [{:text model.label}]))
    (each [_ fact (ipairs (or model.facts []))]
      (append (misa.values.render fact context)))
    (let [lines (misa.layout.wrap-spans [{:spans parts}] columns)
          last (. lines (length lines))]
      (when last
        (let [width (misa.layout.width (table.concat (icollect [_ part (ipairs last.spans)]
                                                       part.text)))]
          (when (< width columns)
            (table.insert last.spans
                          {:text (.. " "
                                     (string.rep "─"
                                                 (math.max 0
                                                           (- columns width 1))))
                           :style :dim
                           :source false}))))
      {: lines})))

(fn build []
  "Declare the shared content boundary renderer."
  (definitions.build :component.group
    [{:catalog :components
      :id :default.group.boundary
      :value {:render boundary}}]
    {:requirements {:component.group [:layout :values.render]}}))

{:build build}
