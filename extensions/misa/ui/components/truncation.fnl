(local definitions (require :misa.definitions))

;; Bound an already laid-out view. Omitted rows are chrome, never source text.
(fn render [model context]
  (let [source (or model.lines [])
        limit (math.max 0 (math.floor (or model.limit (length source))))]
    (var lines [])
    (var total 0)
    (each [_ line (ipairs source)]
      (when (not line.annotation) (set total (+ total 1))))
    (let [hidden (math.max 0 (- total limit))]
      (var index 0)
      (var visible false)
      (each [_ line (ipairs source)]
        (if line.annotation
            (when visible (table.insert lines line))
            (do
              (set index (+ index 1))
              (set visible (if model.tail (> index hidden) (<= index limit)))
              (when visible (table.insert lines line)))))
      (when (> hidden 0)
        (let [notice []]
          (each [_ line (ipairs (misa.layout.wrap-spans [{:omitted_lines hidden
                                                          :spans [{:text (.. "… "
                                                                             hidden
                                                                             (if (= hidden
                                                                                    1)
                                                                                 " line hidden"
                                                                                 " lines hidden"))
                                                                   :style :dim
                                                                   :source false}]}]
                                                        (math.max 1
                                                                  (or context.columns
                                                                      80))
                                                        model.notice_prefix))]
            (table.insert notice line))
          (if model.tail
              (do
                (each [_ line (ipairs lines)] (table.insert notice line))
                (set lines notice))
              (each [_ line (ipairs notice)] (table.insert lines line)))))
      {: lines})))

(fn build []
  "Build the declarations for component truncation."
  (definitions.build :component.truncation
    [{:catalog :components :id :default.content.truncated :value {: render}}]
    {}))

{:build build}
