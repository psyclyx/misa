(local definitions (require :misa.definitions))

;; Generic labeled facts, meters, details, and inline actions. Geometry is
;; measured once across all sections, so every row shares the same columns.
(fn span [text style] {: text :style (or style :value)})
(fn append [target source]
  (each [_ value (ipairs source)] (table.insert target value)))

(fn styled-value [fact context style]
  (icollect [_ part (ipairs (misa.values.render fact context))]
    (misa.patch part {:style (or part.style style :value)})))

(fn finite? [value]
  (and (= (type value) :number) (= value value) (< (math.abs value) math.huge)))

(fn render [model context]
  (local width (math.max 1 (or context.columns 80)))
  (var label-width 0)
  (var value-width 0)
  (each [_ section (ipairs (or model.sections []))]
    (each [_ row (ipairs (or section.rows []))]
      (set label-width
           (math.max label-width (misa.layout.width (or row.label ""))))
      (when (and row.meter row.fact)
        (local value (styled-value row.fact context))
        (set value-width
             (math.max value-width
                       (misa.layout.width (table.concat (icollect [_ part (ipairs value)]
                                                          part.text))))))))
  (set label-width
       (math.min label-width (math.max 1 (math.floor (* width 0.4)))))
  (local meter-width
         (math.max 1 (math.min 24 (- width label-width value-width 4))))
  (local lines [])
  (local by-id (collect [_ action (ipairs (or model.actions []))]
                 action.id
                 action))

  (fn add [spans]
    (append lines (misa.layout.wrap-spans [{: spans}] width)))

  (each [index section (ipairs (or model.sections []))]
    (when (and (> index 1) section.heading)
      (add [(span "")]))
    (when section.title
      (add [(span section.title (if section.heading :dialog.title :label))]))
    (each [_ row (ipairs (or section.rows []))]
      (local label (misa.layout.clip (or row.label "") label-width))
      (local spans [(span (.. label
                              (string.rep " "
                                          (+ 2
                                             (- label-width
                                                (misa.layout.width label)))))
                          :label)])
      (when row.meter
        (local meter row.meter)
        (local ratio
               (when (and (finite? meter.used) (finite? meter.limit)
                          (> meter.limit 0))
                 (math.max 0 (math.min 1 (/ meter.used meter.limit)))))
        (local filled (if ratio (math.floor (+ (* ratio meter-width) 0.5)) 0))
        (table.insert spans (span (string.rep "█" filled) :dialog.title))
        (table.insert spans (span (string.rep "░" (- meter-width filled))
                                  :dim))
        (table.insert spans (span "  ")))
      (when row.fact (append spans (styled-value row.fact context)))
      (when row.actions
        (local actions (icollect [_ id (ipairs row.actions)] (. by-id id)))
        (when (> (length actions) 0)
          (table.insert spans (span "   "))
          (append spans (misa.components.buttons actions model))))
      (add spans)
      (when row.detail
        (local detail [(span (string.rep " " (+ label-width 2)))])
        (append detail (styled-value row.detail context :dim))
        (add detail))))
  {: lines})

(fn []
  "Build the declarations for component data."
  (definitions :component.data
    [{:catalog :components :id :default.data :value {: render}}]
    {:requirements {:component.data [:layout
                                     :components.buttons
                                     :values.render]}}))
