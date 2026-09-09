(local adapter (require :misa.providers.openai))
(local protocol (require :misa.protocols.openai))

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :openai) {}))

(fn transport-settings []
  (adapter.settings (settings)))

{:auth-providers {:openai {:description "OpenAI API key"
                           :discover_models true
                           :id :openai
                           :label :OpenAI
                           :model_provider :openai
                           :strategy :api_key}}
 :serializers {:openai.chat.openai {:accepts (fn [name]
                                               (= name :reasoning_effort))
                                    :serialize (fn [name value]
                                                 (protocol.serialize (transport-settings)
                                                                     name value))}}
 :effects {:provider.openai (fn [effect]
                              (protocol.request (transport-settings)
                                                :openai.chat.openai effect))}
 :events {:provider.openai/discover {:event :models/discover
                                     :priority 7000
                                     :handler (fn [db event]
                                                (when (. (misa.auth.for-model :openai)
                                                         :discover_models)
                                                  (protocol.discover-models (transport-settings)
                                                                            db
                                                                            event)))}
          :provider.openai/models {:event :provider/openai-models
                                   :priority 7000
                                   :handler (fn [db event]
                                              (protocol.models-complete (transport-settings)
                                                                        :openai.chat.openai
                                                                        db event))}
          :provider.openai/complete {:event :provider/openai-complete
                                     :priority 7000
                                     :handler protocol.stream}}}
