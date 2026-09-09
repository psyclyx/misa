;; Status presents usage facts and tracks the current activity label.

(fn total [usage]
  "Return the combined input and output token count."
  (+ (or (and usage usage.input_tokens) 0)
     (or (and usage usage.output_tokens) 0)))

(fn selected [entries id]
  (accumulate [found nil _ model (ipairs (or entries [])) &until found]
    (when (= model.id id) model)))

(fn finite? [value]
  (and (= (type value) :number) (= value value) (< (math.abs value) math.huge)))

(fn plan-value [usage]
  "Describe the least remaining allowance across quota windows."
  (when usage
    (var remaining nil)
    (when (and (= (type usage) :table) (not usage.unavailable))
      (each [_ window (ipairs (or usage.windows []))]
        (let [limit window.limit
              amount (or window.remaining
                         (and (finite? window.used) (finite? limit)
                              (- limit window.used)))]
          (when (and (finite? limit) (> limit 0) (finite? amount)
                     (<= 0 amount limit))
            (let [percent (* 100 (/ amount limit))]
              (set remaining
                   (if remaining (math.min remaining percent) percent)))))))
    (if remaining {:type :percent :value remaining :basis :remaining}
        {:type :unavailable
         :reason (if (and (= (type usage) :table) usage.unavailable)
                     :provider_unavailable
                     :missing_limit)})))

(fn projection [db context]
  "Build semantic status facts from the current database and clock."
  (if (and misa.status misa.status.indicators)
      (misa.status.indicators db context)
      misa.components.render
      (let [presentation (collect [key value (pairs (or context {}))]
                           key
                           value)]
        (when (and misa.animations misa.animations.state)
          (tset presentation :activity_animation
                (misa.animations.state db :status)))
        (. (misa.components.render db :status.indicators
                                   {:indicators [{:id :activity
                                                  :label "●"
                                                  :fact (misa.sub db
                                                                  [:status/activity])}]}
                                   presentation) :lines))
      []))

(fn status-context-value [inputs]
  "Describe the used and available model context."
  (let [used (total (. inputs 1))
        model (selected (. inputs 2) (. inputs 3))]
    (if model
        {:type :ratio : used :limit model.context_window :unit :tokens}
        {:type :tokens :value used})))

(fn initialize []
  "Initialize activity state as ready."
  {:patch {:status (misa.replace {:mode :ready})}})

(fn update [_ event]
  "Record the current activity label."
  {:patch {:status {:mode event.status}}})

{: initialize
 : plan-value
 : projection
 : status-context-value
 : total
 : update}
