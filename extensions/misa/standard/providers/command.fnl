(local command (require :misa.providers.command))

{:models {:command/default {:id :command/default
                            :label :Command
                            :model :default
                            :provider :command}}
 :effects {:provider.command (fn [effect]
                               (let [config (or (. (or (. (misa.configuration)
                                                          :providers)
                                                       {})
                                                   :command)
                                                {})]
                                 (command.request config.argv effect)))}
 :events {:provider.command/complete {:event :provider/command-complete
                                      :handler command.complete}}}
