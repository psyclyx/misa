(local options (require :misa.models.options))

{:events {"request_options/app/start" {:event :app/start
                                       :handler (fn [db event cofx]
                                                  (options.on-app-start cofx.config))
                                       :priority 61000}
          "request_options/request-options/reconcile" {:event :request-options/reconcile
                                                       :handler options.on-request-options-reconcile
                                                       :priority 61000}
          "request_options/request-options/select" {:event :request-options/select
                                                    :handler options.on-request-options-select
                                                    :priority 61000}
          :request_options/model/open {:event :model/open
                                       :handler options.schedule-reconcile
                                       :priority 61000}
          :request_options/model/role {:event :model/role
                                       :handler options.schedule-reconcile
                                       :priority 61000}
          :request_options/model/roles-loaded {:event :model/roles-loaded
                                               :handler options.schedule-reconcile
                                               :priority 61000}
          :request_options/model/select {:event :model/select
                                         :handler options.schedule-reconcile
                                         :priority 61000}
          :request_options/models/provider-availability {:event :models/provider-availability
                                                         :handler options.schedule-reconcile
                                                         :priority 61000}
          :request_options/models/replace-provider {:event :models/replace-provider
                                                    :handler options.schedule-reconcile
                                                    :priority 61000}
          :request_options/models/update {:event :models/update
                                          :handler options.schedule-reconcile
                                          :priority 61000}}
 :services {:request-options.choices options.request-options-choices
            :request-options.prepare options.prepare
            :request-options.reconcile options.reconcile
            :request-options.state options.request-options-state
            :request-options.value (fn [db name]
                                     "Return the selected value for a request option."
                                     (. (options.reconcile db) :values name))}
 :subscriptions {:request-options/indicator {:id :request-options/indicator
                                             :inputs [[:db/path
                                                       :models
                                                       :entries]
                                                      [:db/path
                                                       :models
                                                       :selected]
                                                      [:db/path
                                                       :request_options]]
                                             :compute options.compute-request-options-indicator}}}
