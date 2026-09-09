(local auth (require :misa.providers.auth))

{:commands {:/login {:choice_purpose :auth
                     :completion :auth-provider
                     :description "Log in to a provider"
                     :event :auth/login
                     :name :/login}
            :/logout {:choice_purpose :auth
                      :completion :auth-provider
                      :description "Log out of a provider"
                      :event :auth/logout
                      :name :/logout}
            :/status {:choice_purpose :auth
                      :completion :auth-provider
                      :description "Show provider login state"
                      :event :auth/status
                      :name :/status}}
 :events {:auth.startup {:event :app/start
                         :handler (fn [db]
                                    (auth.startup (misa.auth.providers) db))
                         :priority 11000}
          :auth/provider-status {:event :auth/provider-status
                                 :handler auth.provider-status
                                 :priority 11000}
          :auth/discovery-complete {:event :models/discovery-complete
                                    :handler auth.discovery-complete
                                    :priority 11000}
          :auth/interaction {:event :auth/interaction
                             :handler auth.interaction
                             :priority 11000}
          :auth/dialog-action {:event :auth/dialog-action
                               :handler auth.dialog-action
                               :priority 11000}
          :auth/complete {:event :auth/complete
                          :handler auth.complete
                          :priority 11000}
          :auth/ready {:event :auth/ready
                       :handler (fn []
                                  {:fx [{:type :terminal/read}]})
                       :priority 11000}
          :auth/login {:event :auth/login
                       :priority 11000
                       :handler (fn [db event cofx]
                                  (auth.command {:action :login :name :/login}
                                                db event cofx))}
          :auth/logout {:event :auth/logout
                        :priority 11000
                        :handler (fn [db event cofx]
                                   (auth.command {:action :logout
                                                  :name :/logout}
                                                 db event cofx))}
          :auth/status {:event :auth/status
                        :priority 11000
                        :handler (fn [db event cofx]
                                   (auth.command {:action :status
                                                  :name :/status}
                                                 db event cofx))}}}
