(local status (require :misa.ui.status))
(local indicators (require :misa.ui.status.indicators))

(local default-selections
       [{:id :activity :priority 100 :representation :icon}
        {:id :model :priority 90 :representation :value :hotkey true}
        {:id :effort :priority 70 :hotkey true}
        {:id :session :priority 40 :representation :icon}
        {:id :context :priority 80}
        {:id :plan :priority 60}
        {:id :transcript-detail :priority 20 :hotkey true}])

(local selection-cache (setmetatable {} {:__mode :k}))

(fn selected []
  (let [configured (or (. (or (. (misa.configuration) :status) {}) :indicators)
                       default-selections)
        selections (or (. selection-cache configured)
                       (indicators.selections configured))]
    (tset selection-cache configured selections)
    (icollect [_ selection (ipairs selections)]
      (when (. (misa.catalog :indicators) selection.id) selection))))

{:services {:status.model status.projection
            :status.indicators indicators.status-indicators}
 :indicators {:plan {:id :plan
                     :label :plan
                     :action :usage.open
                     :query [:status/plan]}
              :activity {:id :activity
                         :icon "●"
                         :label :activity
                         :query [:status/activity]}
              :session {:id :session
                        :icon :tok
                        :label :tokens
                        :query [:status/session]}
              :context {:id :context
                        :icon "◫"
                        :label :ctx
                        :query [:status/context]}}
 :subscriptions {:status/activity {:id :status/activity
                                   :inputs [[:db/path :status :mode]]
                                   :compute (fn [inputs]
                                              {:type :activity
                                               :state (or (. inputs 1) :ready)})}
                 :status/session {:id :status/session
                                  :inputs [[:usage/session]]
                                  :compute (fn [inputs]
                                             {:type :tokens
                                              :value (status.total (. inputs 1))})}
                 :status/context {:id :status/context
                                  :inputs [[:usage/last-request]
                                           [:db/path :models :entries]
                                           [:db/path :models :selected]]
                                  :compute status.status-context-value}
                 :status/plan {:id :status/plan
                               :inputs [[:usage/selected-quota]]
                               :compute (fn [inputs]
                                          (status.plan-value (. inputs 1)))}
                 :indicators/model {:id :indicators/model
                                    :inputs (fn []
                                              (icollect [_ selection (ipairs (selected))]
                                                (. (misa.catalog :indicators)
                                                   selection.id :query)))
                                    :compute (fn [inputs]
                                               (indicators.indicators-model-value selected
                                                                                  inputs))}}
 :events {:status/app/start {:event :app/start :handler status.initialize}
          :status/agent/status {:event :agent/status :handler status.update}}
 :validators {:indicators indicators.validate-indicator}}
