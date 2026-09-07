;; Agent facts and their status projections. Provider quota retrieval is owned
;; by providers; this feature consumes normalized usage, never transport records.

(fn total [usage]
  (+ (or (and usage usage.input_tokens) 0) (or (and usage usage.output_tokens) 0)))

(fn selected [entries id]
  (each [_ model (ipairs (or entries []))]
    (when (= model.id id) (lua "return model")))
  nil)

(fn finite [value]
  (and (= (type value) :number) (= value value) (< (math.abs value) math.huge)))

(fn plan-value [usage]
  (when usage
    (var remaining nil)
    (when (and (= (type usage) :table) (not usage.unavailable))
      (each [_ window (ipairs (or usage.windows []))]
        (local limit window.limit)
        (local amount (or window.remaining
                          (and (finite window.used) (finite limit) (- limit window.used))))
        (when (and (finite limit) (> limit 0) (finite amount) (<= 0 amount limit))
          (local percent (* 100 (/ amount limit)))
          (set remaining (if remaining (math.min remaining percent) percent)))))
    (if remaining {:type :percent :value remaining :basis :remaining}
        {:type :unavailable :reason (if (and (= (type usage) :table) usage.unavailable)
                                       :provider_unavailable :missing_limit)})))

(fn usage-patch [event mode]
  {:mode mode
   :usage (when event.usage (misa.replace event.usage))
   :last_usage (when event.last_usage (misa.replace event.last_usage))
   :provider_usage (when event.provider_usage (misa.replace event.provider_usage))})

(fn usage-sections [db]
  (local status (or db.status {}))
  (local usage (or status.usage {}))
  (local last (or status.last_usage {}))
  (local model (and misa.selected_model_projection
                     (misa.selected_model_projection db)))
  (local sections [{:id :model :fields [{:label "Model" :value (or (and model model.label) :none)}
                                       {:label "Context window" :value (and model model.context_window)}]}
                   {:id :session :title "Session usage"
                    :fields [{:label "Input tokens" :value (or usage.input_tokens 0)}
                             {:label "Output tokens" :value (or usage.output_tokens 0)}
                             {:label "Last request" :value (total last)}]}])
  (local providers {})
  (each [provider details (pairs (or db.providers {}))]
    (when (and (= (type details) :table) (or details.subscription_type details.usage))
      (tset providers provider {:plan details.subscription_type :usage details.usage})))
  (each [provider plan (pairs (or status.provider_usage {}))]
    (tset providers provider {:plan (and (. providers provider) (. providers provider :plan)) :usage plan}))
  (local ids (icollect [provider (pairs providers)] provider))
  (table.sort ids)
  (each [_ provider (ipairs ids)]
    (local details (. providers provider))
    (local usage details.usage)
    (local fields [{:label "Plan" :value details.plan}
                   {:label "Plan usage"
                    :value (if (= (type usage) :table)
                               (or usage.summary usage.used
                                   (if (and (not usage.unavailable) (> (length (or usage.windows [])) 0))
                                       "Available" "Unavailable"))
                               (or usage "Unavailable"))}])
    (each [_ window (ipairs (or (and (= (type usage) :table) usage.windows) []))]
      (each [_ field (ipairs [{:key :used :label "used"} {:key :limit :label "limit"}
                              {:key :remaining :label "remaining"} {:key :reset_at :label "resets at"}
                              {:key :reset_at_unix :label "resets at (Unix seconds)"}
                              {:key :reset_after_seconds :label "reset delay (seconds at fetch)"}
                              {:key :status :label "status"}])]
        (when (not= (. window field.key) nil)
          (table.insert fields {:label (.. (or window.label "Quota") " " field.label)
                                :value (. window field.key)}))))
    (each [_ field (ipairs (or (and (= (type usage) :table) usage.fields) []))]
      (table.insert fields field))
    (table.insert sections
                  {:id provider :title provider : fields}))
  sections)


(local indicators
       [{:id :plan :label :plan :action :usage.open :query [:status/plan]}
        {:id :activity :icon "●" :label :activity
         :query [:status/activity]}
        {:id :session :icon :tok :label :tokens
         :query [:status/session]}
        {:id :context :icon "◫" :label :ctx
         :query [:status/context]}])

(fn projection [db context]
  (if misa.indicators_projection (misa.indicators_projection db context)
      misa.render_component
      (let [presentation (collect [key value (pairs (or context {}))] key value)]
        (when misa.animation_presentation
          (tset presentation :activity_animation (misa.animation_presentation db :status)))
        (. (misa.render_component db :status.indicators
                                  {:indicators [{:id :activity :label "●"
                                                 :fact (misa.sub db [:status/activity])}]}
                                  presentation) :lines))
      []))

(local refresh-events {:model/open false :model/select false
                       :models/provider-availability false :models/update false
                       :models/replace-provider false :auth/ready true
                       :transcript/response-end true :transcript/response-interrupted true})

(fn refresh-selected [db event]
  (local model (and misa.selected_model_projection (misa.selected_model_projection db)))
  (local provider (and model model.provider))
  (local previous (and db.status db.status.quota_provider))
  (local changed (not= provider previous))
  (local refresh (and provider (or changed event.force)))
  (when (or changed refresh)
    {:patch {:status {:quota_provider (misa.replace provider)}}
     :fx (if refresh [{:type :dispatch :event {:type :usage/refresh : provider}}] [])}))

{:setup (fn []
          (local fx
                 [{:type :register/sub
                   :value {:id :status/activity :inputs [[:db/path :status :mode]]
                           :compute (fn [inputs] {:type :activity :state (or (. inputs 1) :ready)})}}
                  {:type :register/sub
                   :value {:id :status/session :inputs [[:db/path :status :usage]]
                           :compute (fn [inputs] {:type :tokens :value (total (. inputs 1))})}}
                  {:type :register/sub
                   :value {:id :status/context
                           :inputs [[:db/path :status :last_usage] [:db/path :models :entries]
                                    [:db/path :models :selected]]
                           :compute (fn [inputs]
                                      (local used (total (. inputs 1)))
                                      (local model (selected (. inputs 2) (. inputs 3)))
                                      (if model {:type :ratio : used :limit model.context_window :unit :tokens}
                                          {:type :tokens :value used}))}}
                  {:type :register/sub
                   :value {:id :status/plan
                           :inputs [[:db/path :models :entries] [:db/path :models :selected]
                                    [:db/path :status :provider_usage] [:db/path :providers]]
                           :compute (fn [inputs]
                                      (local model (selected (. inputs 1) (. inputs 2)))
                                      (local provider (and model model.provider))
                                      (local overrides (. inputs 3))
                                      (local providers (. inputs 4))
                                      (plan-value (and provider
                                                       (or (and overrides (. overrides provider))
                                                           (and providers (. providers provider)
                                                                (. providers provider :usage))))))}}
                  {:type :register/event :name :usage/check-selected :handler refresh-selected}
                  {:type :register/event :name :app/start
                   :handler (fn []
                              {:patch {:status (misa.replace {:last_usage {} :mode :ready :provider_usage {}
                                                              :usage {:input_tokens 0 :output_tokens 0}})}
                               :fx [{:type :dispatch :event {:type :usage/check-selected}}]})}
                  {:type :register/event :name :agent/status
                   :handler (fn [_ event]
                              {:patch {:status (usage-patch event event.status)}})}
                  {:type :register/event :name :agent/usage
                   :handler (fn [_ event] {:patch {:status (usage-patch event)}})}
                  {:type :register/service :name :status_projection :value projection}
                  {:type :register/action
                   :value {:id :usage.open :label "Show usage" :event {:type :usage/open}
                           :available (fn [db] (not db.dialog))}}
                  {:type :register/command
                   :value {:choice_purpose :command :description "Show token and coding-plan usage"
                           :event :usage/open :name :/usage}}
                  {:type :register/event :name :usage/open
                   :handler (fn [db]
                              {:fx [{:type :dispatch
                                     :event {:type :dialog/open :id :usage :title "Usage"
                                             :sections (usage-sections db)
                                             :actions [{:id :close :label "Close" :primary true}]
                                             :cancellable true :completion :usage/close :correlation :usage}}
                                    {:type :dispatch :event {:type :usage/refresh}}]})}
                  {:type :register/event :name :usage/updated
                   :handler (fn [db]
                              (when (and db.dialog (= db.dialog.id :usage))
                                {:fx [{:type :dispatch :event {:type :dialog/update :id :usage
                                                              :correlation :usage :sections (usage-sections db)}}]}))}
                  {:type :register/event :name :usage/close :handler (fn [] nil)}])
          ;; The queued check observes the completed model transaction, even
          ;; when this extension registers before the model owner.
          (each [name force (pairs refresh-events)]
            (table.insert fx {:type :register/event : name
                              :handler (fn []
                                         {:fx [{:type :dispatch
                                                :event {:type :usage/check-selected : force}}]})}))
          (when (misa.has_setup_effect :register/indicator)
            (each [_ value (ipairs indicators)]
              (table.insert fx {:type :register/indicator : value})))
          {: fx})}
