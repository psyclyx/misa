(local adapter (require :misa.providers.anthropic))
(local protocol (require :misa.protocols.anthropic))

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :anthropic) {}))

(fn transport-settings []
  (adapter.settings (settings)))

{:auth-providers {:anthropic {:description "Anthropic API key"
                              :discover_models true
                              :id :anthropic
                              :label :Anthropic
                              :model_provider :anthropic
                              :strategy :api_key}}
 :serializers {:anthropic.messages.anthropic {:accepts (fn [name]
                                                         (= name
                                                            :reasoning_effort))
                                              :serialize protocol.serialize}}
 :effects {:provider.anthropic (fn [effect]
                                 (protocol.request (transport-settings)
                                                   :anthropic.messages.anthropic
                                                   effect))}
 :events {:provider.anthropic/discover {:event :models/discover
                                        :priority 4000
                                        :handler (fn [db event]
                                                   (when (. (misa.auth.for-model :anthropic)
                                                            :discover_models)
                                                     (protocol.discover-models (transport-settings)
                                                                               db
                                                                               event)))}
          :provider.anthropic/models {:event :provider/anthropic-models
                                      :priority 4000
                                      :handler (fn [db event]
                                                 (protocol.page (transport-settings)
                                                                :anthropic.messages.anthropic
                                                                db event))}
          :provider.anthropic/complete {:event :provider/anthropic-complete
                                        :priority 4000
                                        :handler (fn [db event]
                                                   (protocol.stream :anthropic
                                                                    db event))}}}
