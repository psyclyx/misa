(local adapter (require :misa.providers.openrouter))
(local options (require :misa.providers.openai-options))
(local protocol (require :misa.protocols.openai))

(local serializer (options.compose [adapter.routing adapter.reasoning options.standard]))

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :openrouter) {}))

(fn transport-settings []
  (adapter.settings (settings)))

{:auth-providers {:openrouter {:description "OpenRouter OAuth"
                               :discover_models true
                               :id :openrouter
                               :label :OpenRouter
                               :model_provider :openrouter
                               :profile {:id :default}
                               :strategy :loopback_pkce}}
 :serializers {:openai.chat.openrouter serializer}
 :effects {:provider.openrouter (fn [effect]
                                  (protocol.request (transport-settings)
                                                    :openai.chat.openrouter
                                                    effect))}
 :events {:provider.openrouter/discover {:event :models/discover
                                         :priority 8000
                                         :handler (fn [db event]
                                                    (when (. (misa.auth.for-model :openrouter)
                                                             :discover_models)
                                                      (protocol.discover-models (transport-settings)
                                                                                db
                                                                                event)))}
          :provider.openrouter/models {:event :provider/openrouter-models
                                       :priority 8000
                                       :handler (fn [db event]
                                                  (protocol.models-complete (transport-settings)
                                                                            :openai.chat.openrouter
                                                                            db
                                                                            event))}
          :provider.openrouter/complete {:event :provider/openrouter-complete
                                         :priority 8000
                                         :handler protocol.stream}}}
