(fn usage-patch [event]
  {:session (when event.usage (misa.replace event.usage))
   :last_request (when event.last_usage (misa.replace event.last_usage))
   :providers (when event.provider_usage (misa.replace event.provider_usage))})

(fn refresh-selected [db event]
  "Refresh quota data when the selected provider changes."
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
  "Initialize request and provider usage state."
  {:patch {:usage (misa.replace {:session {:input_tokens 0 :output_tokens 0}
                                 :last_request {}
                                 :providers {}})}
   :fx [{:type :dispatch :event {:type :usage/check-selected}}]})

(fn record-usage [_ event]
  "Capture normalized usage facts from an agent event."
  {:patch {:usage (usage-patch event)}})

(fn queue-refresh [force]
  "Schedule a quota check after the current transaction."
  {:fx [{:type :dispatch :event {:type :usage/check-selected : force}}]})

(fn selected-quota [inputs]
  "Select quota facts for the current model's provider."
  (let [provider (accumulate [found nil _ model (ipairs (or (. inputs 1) []))]
                   (if (= model.id (. inputs 2)) model.provider found))]
    (when provider
      (or (. (or (. inputs 3) {}) provider)
          (. (or (. (or (. inputs 4) {}) provider) {}) :usage)))))

{: refresh-selected : start : record-usage : queue-refresh : selected-quota}
