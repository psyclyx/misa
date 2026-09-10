;; Factory for user-configured OpenAI-compatible providers.  It deliberately
;; takes settings as an argument, so a configuration can add any number of
;; providers without changing Misa's shipped catalog.
(local protocol (require :misa.protocols.openai))
(local options (require :misa.providers.openai-options))

(fn trim-slash [url] (url:gsub "/+$" ""))

(fn api-key [base provision-url]
  "Declare an API-key flow and an optional key-provisioning page."
  {:profile {:api_base base :provision_url provision-url} :strategy :api_key})

(fn device-oauth [base authorization-url token-url client-id]
  "Declare standard OAuth device authorization for a configured provider."
  {:profile {:api_base base
             :authorization_url authorization-url
             :id client-id
             :token_url token-url}
   :strategy :device_oauth})

(fn provider [id config]
  "Create one API-key provider from {api :openai :base_url ...}."
  (assert (= (type id) :string) "generic provider ID must be a string")
  (assert (not= (id:match :^generic/.+) nil)
          "generic provider IDs must begin with generic/")
  (assert (= config.api :openai)
          "generic providers currently support api: openai")
  (assert (= (type config.base_url) :string)
          "generic provider base_url is required")
  (let [base (trim-slash config.base_url)
        serializer (.. :openai.chat. id)
        transport {:catalogue_authoritative true
                   :credential id
                   : id
                   :max_tokens config.max_tokens
                   :models_url (or config.models_url (.. base :/models))
                   :timeouts config.timeouts
                   :url (or config.url (.. base :/chat/completions))}
        flow (or config.auth (api-key base config.provision_url))
        auth {:description (or config.description "OpenAI-compatible API key")
              :discover_models (not= config.discover_models false)
              : id
              :label (or config.label id)
              :model_provider id
              ;; Native code persists this base with the credential and pins requests to it.
              :profile flow.profile
              :strategy flow.strategy}]
    {:auth-providers {id auth}
     :serializers {serializer (options.compose [options.standard])}
     :effects {(.. :provider. id) (fn [effect]
                                    (protocol.request transport serializer
                                                      effect))}
     :events {(.. :provider. id :/discover) {:event :models/discover
                                             :priority 7600
                                             :handler (fn [db event]
                                                        (when (and (or (not event.provider)
                                                                       (= event.provider
                                                                          id))
                                                                   (. (misa.auth.for-model id)
                                                                      :discover_models))
                                                          (protocol.discover-models transport
                                                                                    db
                                                                                    event)))}
              (.. :provider. id :/models) {:event (.. :provider/ id :-models)
                                           :priority 7600
                                           :handler (fn [db event]
                                                      (protocol.models-complete transport
                                                                                serializer
                                                                                db
                                                                                event))}
              (.. :provider. id :/complete) {:event (.. :provider/ id
                                                        :-complete)
                                             :priority 7600
                                             :handler protocol.stream}}}))

{: api-key : device-oauth : provider}
