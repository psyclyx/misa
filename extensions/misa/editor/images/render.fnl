;; A semantic image rectangle, with a readable fallback on text-only terminals.

(fn render-image [model context]
  "Render image content."
  (let [title (.. (: (: (or model.name :Image) :gsub "[%z\001-\031\127]" " ")
                     :gsub "\194[\128-\159]" " ") "  "
                  (tostring (or model.width "?")) "×"
                  (tostring (or model.height "?")))
        lines [{:spans [{:style :label
                         :text (misa.layout.clip title (or context.columns 80))}]}]
        preview model.preview]
    (when (and context.images preview)
      (var columns (math.max 1 (math.min (or context.columns 80) 60)))
      (let [rows (math.max 1
                           (math.min (or context.max_image_rows 10)
                                     (math.ceil (/ (/ (* columns preview.height)
                                                      preview.width)
                                                   2))))]
        (set columns
             (math.max 1
                       (math.min columns
                                 (math.floor (/ (* rows 2 preview.width)
                                                preview.height)))))
        (for [i 1 rows]
          (tset lines (+ (length lines) 1)
                {:image_row model.image_id :spans {}}))
        (tset (. lines 2) :image
              {: columns
               :data preview.data
               :format :rgba
               :height preview.height
               :id model.image_id
               : rows
               :width preview.width})))
    {: lines}))

{: render-image}
