(local deepseek (require :misa.providers.deepseek))
(local protocol (require :misa.protocols.openai))

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :deepseek) {}))

(fn transport-settings []
  (deepseek.settings (settings)))

(fn catalogue [db event]
  "Merge published DeepSeek facts onto discovered catalogue rows."
  (when (or (not event.provider) (= event.provider :deepseek))
    {:fx [{:type :dispatch
           :event {:models (deepseek.enrichment)
                   :provider :deepseek
                   :type :models/update}}]}))

{:auth-providers {:deepseek {:description "DeepSeek API key"
                             :discover_models true
                             :id :deepseek
                             :label :DeepSeek
                             :model_provider :deepseek
                             :profile {:provision_url "https://platform.deepseek.com/api_keys"}
                             :strategy :api_key}}
 :serializers {:openai.chat.deepseek deepseek.serializer}
 :effects {:provider.deepseek (fn [effect]
                                (protocol.request (transport-settings)
                                                  :openai.chat.deepseek effect))}
 :events {:provider.deepseek/discover {:event :models/discover
                                       :handler (fn [db event]
                                                  (when (. (misa.auth.for-model :deepseek)
                                                           :discover_models)
                                                    (protocol.discover-models (transport-settings)
                                                                              db
                                                                              event)))
                                       :priority 7500}
          :provider.deepseek/models {:event :provider/deepseek-models
                                     :handler (fn [db event]
                                                (protocol.models-complete (transport-settings)
                                                                          :openai.chat.deepseek
                                                                          db
                                                                          event))
                                     :priority 7500}
          :provider.deepseek/complete {:event :provider/deepseek-complete
                                       :handler protocol.stream
                                       :priority 7500}
          :provider.deepseek/catalogue {:event :models/discovery-complete
                                        :handler catalogue
                                        :priority 7500}}}
