;; Shared wiring for API-key providers that implement OpenAI chat-completions.
(local protocol (require :misa.protocols.openai))
(local options (require :misa.providers.openai-options))

(fn preset [id label base priority provision-url]
  "Create an OpenAI-compatible API-key provider preset."
  (let [serializer (.. :openai.chat. id)]
    (fn transport []
      (let [config (or (. (or (. (misa.configuration) :providers) {}) id) {})]
        {:catalogue_authoritative true
         :credential id
         : id
         :max_tokens config.max_tokens
         :models_url (or config.models_url (.. base :/models))
         :timeouts config.timeouts
         :url (or config.url (.. base :/chat/completions))}))

    {:auth-providers {id {:description (.. label " API key")
                          :discover_models true
                          : id
                          : label
                          :model_provider id
                          :profile (when provision-url
                                     {:provision_url provision-url})
                          :strategy :api_key}}
     :serializers {serializer (options.compose [options.standard])}
     :effects {(.. :provider. id) (fn [effect]
                                    (protocol.request (transport) serializer
                                                      effect))}
     :events {(.. :provider. id :/discover) {:event :models/discover
                                             : priority
                                             :handler (fn [db event]
                                                        (when (. (misa.auth.for-model id)
                                                                 :discover_models)
                                                          (protocol.discover-models (transport)
                                                                                    db
                                                                                    event)))}
              (.. :provider. id :/models) {:event (.. :provider/ id :-models)
                                           : priority
                                           :handler (fn [db event]
                                                      (protocol.models-complete (transport)
                                                                                serializer
                                                                                db
                                                                                event))}
              (.. :provider. id :/complete) {:event (.. :provider/ id
                                                        :-complete)
                                             : priority
                                             :handler protocol.stream}}}))

{: preset}
