(local adapter (require :misa.providers.kimi))
(local protocol (require :misa.protocols.anthropic))

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :kimi) {}))

(fn endpoint-profile []
  (. (assert (misa.auth.provider :kimi-coding)
             "Kimi authentication is not configured") :profile))

(fn transport-settings []
  (adapter.settings (settings) (endpoint-profile)))

{:auth-providers {:kimi-coding {:description "Kimi coding plan OAuth (global)"
                                :discover_models true
                                :id :kimi-coding
                                :label "Kimi Coding"
                                :model_provider :kimi
                                :profile adapter.profiles.global
                                :strategy :device_oauth}}
 :serializers {:anthropic.messages.kimi {:accepts (fn [name]
                                                    (= name :reasoning_effort))
                                         :serialize protocol.serialize}}
 :effects {:provider.kimi (fn [effect]
                            (protocol.request (transport-settings)
                                              :anthropic.messages.kimi effect))}
 :events {:provider.kimi/discover {:event :models/discover
                                   :priority 5000
                                   :handler (fn [db event]
                                              (when (. (misa.auth.for-model :kimi)
                                                       :discover_models)
                                                (protocol.discover-models (transport-settings)
                                                                          db
                                                                          event)))}
          :provider.kimi/models {:event :provider/kimi-models
                                 :priority 5000
                                 :handler (fn [db event]
                                            (protocol.page (transport-settings)
                                                           :anthropic.messages.kimi
                                                           db event))}
          :provider.kimi/complete {:event :provider/kimi-complete
                                   :priority 5000
                                   :handler (fn [db event]
                                              (protocol.stream :kimi db event))}
          :provider.kimi/usage-refresh {:event :usage/refresh
                                        :priority 5000
                                        :handler (fn [db event]
                                                   (adapter.refresh-usage (endpoint-profile)
                                                                          (settings)
                                                                          db
                                                                          event))}
          :provider.kimi/usage-complete {:event :provider/kimi-usage
                                         :priority 5000
                                         :handler adapter.receive-usage}}}
