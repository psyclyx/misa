(local definitions (require :misa.definitions))

;; Status presents usage facts and tracks the current activity label.

(fn total [usage]
  (+ (or (and usage usage.input_tokens) 0)
     (or (and usage usage.output_tokens) 0)))

(fn selected [entries id]
  (accumulate [found nil _ model (ipairs (or entries [])) &until found]
    (when (= model.id id) model)))

(fn finite? [value]
  (and (= (type value) :number) (= value value) (< (math.abs value) math.huge)))

(fn plan-value [usage]
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

(local indicators [{:id :plan
                    :label :plan
                    :action :usage.open
                    :query [:status/plan]}
                   {:id :activity
                    :icon "●"
                    :label :activity
                    :query [:status/activity]}
                   {:id :session
                    :icon :tok
                    :label :tokens
                    :query [:status/session]}
                   {:id :context
                    :icon "◫"
                    :label :ctx
                    :query [:status/context]}])

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
  (let [used (total (. inputs 1))
        model (selected (. inputs 2) (. inputs 3))]
    (if model
        {:type :ratio : used :limit model.context_window :unit :tokens}
        {:type :tokens :value used})))

(fn build []
  "Build the declarations for status."
  (let [fx [(let [definition {:id :status/activity
                              :inputs [[:db/path :status :mode]]
                              :compute (fn [inputs]
                                         {:type :activity
                                          :state (or (. inputs 1) :ready)})}]
              {:catalog :subscriptions
               :id (. definition :id)
               :value definition})
            (let [definition {:id :status/session
                              :inputs [[:usage/session]]
                              :compute (fn [inputs]
                                         {:type :tokens
                                          :value (total (. inputs 1))})}]
              {:catalog :subscriptions
               :id (. definition :id)
               :value definition})
            (let [definition {:id :status/context
                              :inputs [[:usage/last-request]
                                       [:db/path :models :entries]
                                       [:db/path :models :selected]]
                              :compute status-context-value}]
              {:catalog :subscriptions
               :id (. definition :id)
               :value definition})
            (let [definition {:id :status/plan
                              :inputs [[:usage/selected-quota]]
                              :compute (fn [inputs] (plan-value (. inputs 1)))}]
              {:catalog :subscriptions
               :id (. definition :id)
               :value definition})
            {:catalog :events
             :value {:event :app/start
                     :handler (fn []
                                {:patch {:status (misa.replace {:mode :ready})}})}}
            {:catalog :events
             :value {:event :agent/status
                     :handler (fn [_ event]
                                {:patch {:status {:mode event.status}}})}}
            {:catalog :services :id :status.model :value projection}]]
    (each [_ value (ipairs indicators)]
      (table.insert fx (let [definition value]
                         {:catalog :indicators
                          :id (. definition :id)
                          :value definition})))
    (definitions.build :status fx {})))

{:build build}
