(local components (require :misa.ui.components))

(fn settings [config] (or config.components {}))

{:services {:components.lookup components.components-lookup
            :components.resolve components.components-resolve
            :components.render components.components-render
            :components.project components.components-project
            :components.entry components.components-entry
            :components.swap components.components-swap}
 :subscriptions {:components/projection {:id :components/projection
                                         :inputs [[:db/path :db]
                                                  [:db/path :items]
                                                  [:db/path :context]]
                                         :compute components.components-projection-value}}
 :events {:components/app/start {:event :app/start
                                 :handler (fn [db _ cofx]
                                            (let [config (settings cofx.config)]
                                              (components.app-start config
                                                                    (or config.roles
                                                                        config)
                                                                    db)))}
          :components/components/loaded {:event :components/loaded
                                         :handler components.components-loaded}
          :components/components/swap {:event :components/swap
                                       :handler (fn [db event cofx]
                                                  (components.components-swap-handler (settings cofx.config)
                                                                                      db
                                                                                      event))}}
 :validators {:components components.validate-component}}
