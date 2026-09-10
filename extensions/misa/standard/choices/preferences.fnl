(local {: on-app-start
        : on-choice-used
        : on-preferences-toggle
        : use
        : on-preferences-loaded} (require :misa.choices.preferences))

{:events {:preferences/app/start {:event :app/start
                                  :handler on-app-start
                                  :priority 22000}
          :preferences/preferences/loaded {:event :preferences/loaded
                                           :handler (fn [db event cofx]
                                                      (on-preferences-loaded cofx.config
                                                                             db
                                                                             event))
                                           :priority 22000}
          :preferences/choice/used {:event :choice/used
                                    :handler on-choice-used
                                    :priority 22000}
          :preferences/preferences/toggle {:event :preferences/toggle
                                           :handler on-preferences-toggle
                                           :priority 22000}}
 :services {:preferences.use use}}
