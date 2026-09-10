(local costs (require :misa.costs))
(fn pricing [config]
  (costs.overrides (or config.costs {})))

(fn patch-event [transition]
  (fn [db event cofx]
    (let [patch (transition db event cofx)]
      (when patch {: patch}))))

(fn complete [db event cofx]
  (when db.costs (costs.complete (pricing cofx.config) db event)))

{:services {: costs.estimate
            :costs.model (fn [db id]
                           "Return pricing and availability for a model."
                           (costs.costs-model (pricing (misa.configuration)) db
                                              id))
            :costs.group costs.costs-group
            :costs.response costs.costs-response}
 :subscriptions {:costs/responses {:inputs [[:db/path :costs :responses]]
                                   :compute costs.costs-responses}
                 :costs/response {:inputs [[:costs/responses]]
                                  :compute costs.response-value}
                 :costs/groups {:inputs [[:db/path :costs :responses]]
                                :compute costs.costs-groups}
                 :costs/total {:inputs [[:db/path :costs :responses]]
                               :compute costs.costs-total}
                 :costs/indicator {:inputs [[:costs/total]]
                                   :compute costs.costs-indicator}}
 :indicators {:cost {:icon "$"
                     :id :cost
                     :label :cost
                     :query [:costs/indicator]}}
 :events {:costs/app/start {:event :app/start
                            :priority 59000
                            :handler (patch-event costs.reset)}
          :costs/transcript/reset {:event :transcript/reset
                                   :priority 59000
                                   :handler (patch-event costs.reset)}
          :costs/transcript/response-start {:event :transcript/response-start
                                            :priority 59000
                                            :handler (patch-event (fn [db
                                                                       event
                                                                       cofx]
                                                                    (costs.start-response (pricing cofx.config)
                                                                                          db
                                                                                          event)))}
          :costs/transcript/response-end {:event :transcript/response-end
                                          :priority 59000
                                          :handler (patch-event complete)}
          :costs/tool-summary/usage {:event :tool-summary/usage
                                     :priority 59000
                                     :handler (patch-event complete)}
          :costs/compaction/usage {:event :compaction/usage
                                   :priority 59000
                                   :handler (patch-event complete)}
          :costs/transcript/response-interrupted {:event :transcript/response-interrupted
                                                  :priority 59000
                                                  :handler (patch-event costs.interrupt-response)}}}
