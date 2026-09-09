(local definitions (require :misa.definitions))

(fn usage-patch [event]
  {:session (when event.usage (misa.replace event.usage))
   :last_request (when event.last_usage (misa.replace event.last_usage))
   :providers (when event.provider_usage (misa.replace event.provider_usage))})

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
  (let [model (and misa.models misa.models.selected (misa.models.selected db))
        provider (and model model.provider)
        previous (and db.usage db.usage.quota_provider)
        changed (not= provider previous)
        refresh (and provider (or changed event.force))]
    (when (or changed refresh)
      {:patch {:usage {:quota_provider (misa.replace provider)}}
       :fx (if refresh
               [{:type :dispatch :event {:type :usage/refresh : provider}}]
               [])})))

(fn start []
  {:patch {:usage (misa.replace {:session {:input_tokens 0 :output_tokens 0}
                                 :last_request {}
                                 :providers {}})}
   :fx [{:type :dispatch :event {:type :usage/check-selected}}]})

(fn record-usage [_ event]
  {:patch {:usage (usage-patch event)}})

(fn queue-refresh [_ event]
  {:fx [{:type :dispatch
         :event {:type :usage/check-selected
                 :force (. refresh-events event.type)}}]})

(fn selected-quota [inputs]
  (let [provider (accumulate [found nil _ model (ipairs (or (. inputs 1) []))]
                   (if (= model.id (. inputs 2)) model.provider found))]
    (when provider
      (or (. (or (. inputs 3) {}) provider)
          (. (or (. (or (. inputs 4) {}) provider) {}) :usage)))))

(fn build []
  "Collect request usage and refresh the selected provider's quotas."
  (let [declarations [{:catalog :subscriptions
                       :id :usage/session
                       :value {:inputs [[:db/path :usage :session]]
                               :compute (fn [inputs] (. inputs 1))}}
                      {:catalog :subscriptions
                       :id :usage/last-request
                       :value {:inputs [[:db/path :usage :last_request]]
                               :compute (fn [inputs] (. inputs 1))}}
                      {:catalog :subscriptions
                       :id :usage/providers
                       :value {:inputs [[:db/path :usage :providers]]
                               :compute (fn [inputs] (. inputs 1))}}
                      {:catalog :subscriptions
                       :id :usage/selected-quota
                       :value {:inputs [[:db/path :models :entries]
                                        [:db/path :models :selected]
                                        [:usage/providers]
                                        [:db/path :providers]]
                               :compute selected-quota}}
                      {:catalog :events
                       :value {:event :usage/check-selected
                               :handler refresh-selected}}
                      {:catalog :events
                       :value {:event :app/start :handler start}}]]
    (each [_ name (ipairs [:agent/status :agent/usage])]
      (table.insert declarations
                    {:catalog :events
                     :value {:event name :handler record-usage}}))
    ;; Queue the check so it observes the completed model transaction.
    (each [name force (pairs refresh-events)]
      (table.insert declarations
                    {:catalog :events
                     :value {:event name :handler queue-refresh}}))
    (definitions.build :usage.lifecycle declarations {})))

{: build}
