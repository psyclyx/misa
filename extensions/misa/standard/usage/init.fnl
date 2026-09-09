(local usage (require :misa.usage))
(local refresh-events
       {:model/open false
        :model/select false
        :models/provider-availability false
        :models/update false
        :models/replace-provider false
        :auth/ready true
        :transcript/response-end true
        :transcript/response-interrupted true})

(local catalog
       {:subscriptions {:usage/session {:inputs [[:db/path :usage :session]]
                                        :compute (fn [inputs] (. inputs 1))}
                        :usage/last-request {:inputs [[:db/path
                                                       :usage
                                                       :last_request]]
                                             :compute (fn [inputs] (. inputs 1))}
                        :usage/providers {:inputs [[:db/path :usage :providers]]
                                          :compute (fn [inputs] (. inputs 1))}
                        :usage/selected-quota {:inputs [[:db/path
                                                         :models
                                                         :entries]
                                                        [:db/path
                                                         :models
                                                         :selected]
                                                        [:usage/providers]
                                                        [:db/path :providers]]
                                               :compute usage.selected-quota}}
        :events {:usage.lifecycle/usage/check-selected {:event :usage/check-selected
                                                        :priority 54001
                                                        :handler usage.refresh-selected}
                 :usage.lifecycle/app/start {:event :app/start
                                             :priority 54001
                                             :handler usage.start}
                 :usage.lifecycle/agent/status {:event :agent/status
                                                :priority 54001
                                                :handler usage.record-usage}
                 :usage.lifecycle/agent/usage {:event :agent/usage
                                               :priority 54001
                                               :handler usage.record-usage}}})

(each [event force (pairs refresh-events)]
  (tset catalog.events (.. "usage.lifecycle/" event)
        {: event :priority 54001 :handler (fn [] (usage.queue-refresh force))}))

catalog
