(local definitions (require :misa.definitions))

;; Agent facts and their status projections. Provider quota retrieval is owned
;; by providers; this feature consumes normalized usage, never transport records.

(fn total [usage]
  (+ (or (and usage usage.input_tokens) 0)
     (or (and usage usage.output_tokens) 0)))

(fn selected [entries id]
  (each [_ model (ipairs (or entries []))]
    (when (= model.id id) (lua "return model")))
  nil)

(fn finite? [value]
  (and (= (type value) :number) (= value value) (< (math.abs value) math.huge)))

(fn plan-value [usage]
  (when usage
    (var remaining nil)
    (when (and (= (type usage) :table) (not usage.unavailable))
      (each [_ window (ipairs (or usage.windows []))]
        (local limit window.limit)
        (local amount (or window.remaining
                          (and (finite? window.used) (finite? limit)
                               (- limit window.used))))
        (when (and (finite? limit) (> limit 0) (finite? amount)
                   (<= 0 amount limit))
          (local percent (* 100 (/ amount limit)))
          (set remaining (if remaining (math.min remaining percent) percent)))))
    (if remaining {:type :percent :value remaining :basis :remaining}
        {:type :unavailable
         :reason (if (and (= (type usage) :table) usage.unavailable)
                     :provider_unavailable
                     :missing_limit)})))

(fn usage-patch [event mode]
  {:mode mode
   :usage (when event.usage (misa.replace event.usage))
   :last_usage (when event.last_usage (misa.replace event.last_usage))
   :provider_usage (when event.provider_usage
                     (misa.replace event.provider_usage))})

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

(local refresh-events
       {:model/open false
        :model/select false
        :models/provider-availability false
        :models/update false
        :models/replace-provider false
        :auth/ready true
        :transcript/response-end true
        :transcript/response-interrupted true})

(fn refresh-selected [db event]
  (local model (and misa.models misa.models.selected (misa.models.selected db)))
  (local provider (and model model.provider))
  (local previous (and db.status db.status.quota_provider))
  (local changed (not= provider previous))
  (local refresh (and provider (or changed event.force)))
  (when (or changed refresh)
    {:patch {:status {:quota_provider (misa.replace provider)}}
     :fx (if refresh
             [{:type :dispatch :event {:type :usage/refresh : provider}}]
             [])}))

(fn []
  "Build the declarations for status."
  (local fx
         [(let [definition {:id :status/activity
                            :inputs [[:db/path :status :mode]]
                            :compute (fn [inputs]
                                       {:type :activity
                                        :state (or (. inputs 1) :ready)})}]
            {:catalog :subscriptions :id (. definition :id) :value definition})
          (let [definition {:id :status/session
                            :inputs [[:db/path :status :usage]]
                            :compute (fn [inputs]
                                       {:type :tokens
                                        :value (total (. inputs 1))})}]
            {:catalog :subscriptions :id (. definition :id) :value definition})
          (let [definition {:id :status/context
                            :inputs [[:db/path :status :last_usage]
                                     [:db/path :models :entries]
                                     [:db/path :models :selected]]
                            :compute (fn [inputs]
                                       (local used (total (. inputs 1)))
                                       (local model
                                              (selected (. inputs 2)
                                                        (. inputs 3)))
                                       (if model
                                           {:type :ratio
                                            : used
                                            :limit model.context_window
                                            :unit :tokens}
                                           {:type :tokens :value used}))}]
            {:catalog :subscriptions :id (. definition :id) :value definition})
          (let [definition {:id :status/plan
                            :inputs [[:db/path :models :entries]
                                     [:db/path :models :selected]
                                     [:db/path :status :provider_usage]
                                     [:db/path :providers]]
                            :compute (fn [inputs]
                                       (local model
                                              (selected (. inputs 1)
                                                        (. inputs 2)))
                                       (local provider
                                              (and model model.provider))
                                       (local overrides (. inputs 3))
                                       (local providers (. inputs 4))
                                       (plan-value (and provider
                                                        (or (and overrides
                                                                 (. overrides
                                                                    provider))
                                                            (and providers
                                                                 (. providers
                                                                    provider)
                                                                 (. providers
                                                                    provider
                                                                    :usage))))))}]
            {:catalog :subscriptions :id (. definition :id) :value definition})
          {:catalog :events
           :value {:event :usage/check-selected :handler refresh-selected}}
          {:catalog :events
           :value {:event :app/start
                   :handler (fn []
                              {:patch {:status (misa.replace {:last_usage {}
                                                              :mode :ready
                                                              :provider_usage {}
                                                              :usage {:input_tokens 0
                                                                      :output_tokens 0}})}
                               :fx [{:type :dispatch
                                     :event {:type :usage/check-selected}}]})}}
          {:catalog :events
           :value {:event :agent/status
                   :handler (fn [_ event]
                              {:patch {:status (usage-patch event event.status)}})}}
          {:catalog :events
           :value {:event :agent/usage
                   :handler (fn [_ event]
                              {:patch {:status (usage-patch event)}})}}
          {:catalog :services :id :status.model :value projection}])
  ;; The queued check observes the completed model transaction, even
  ;; when this extension registers before the model owner.
  (each [name force (pairs refresh-events)]
    (table.insert fx
                  {:catalog :events
                   :value {:event name
                           :handler (fn []
                                      {:fx [{:type :dispatch
                                             :event {:type :usage/check-selected
                                                     : force}}]})}}))
  (do
    (each [_ value (ipairs indicators)]
      (table.insert fx (let [definition value]
                         {:catalog :indicators
                          :id (. definition :id)
                          :value definition}))))
  (definitions :status fx {}))
