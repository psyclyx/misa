;; Kimi Code subscription/API provider over Anthropic Messages.

{:setup (fn [context]
          (assert (and misa.protocols misa.protocols.anthropic)
                  "provider.kimi requires protocol.anthropic first")
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table) providers.kimi) nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local profiles
                 {:global {:api_base "https://api.kimi.ai/coding/v1"
                           :authorization_url "https://auth.kimi.ai/api/oauth/device_authorization"
                           :id :global
                           :token_url "https://auth.kimi.ai/api/oauth/token"}
                  :mainland {:api_base "https://api.kimi.com/coding/v1"
                             :authorization_url "https://auth.kimi.com/api/oauth/device_authorization"
                             :id :mainland
                             :token_url "https://auth.kimi.com/api/oauth/token"}})
          (local region (or config.region :global))
          (local profile
                 (assert (. profiles region)
                         "providers.kimi.region must be global or mainland"))
          (local models-url (or config.models_url
                                (or (and (and (not= config.discover_models
                                                    false)
                                              (= config.models nil))
                                         (.. profile.api_base :/models))
                                    nil)))
          (misa.reg_auth_provider {:description (.. "Kimi coding plan OAuth ("
                                                    region ")")
                                   :discover_models (not= models-url nil)
                                   :id :kimi-coding
                                   :label "Kimi Coding"
                                   :model_provider :kimi
                                   : profile
                                   :strategy :device_oauth})
          (misa.protocols.anthropic {:auth_header :authorization
                                     :auth_prefix "Bearer "
                                     :catalogue_authoritative true
                                     :credential :kimi-coding
                                     :headers [{:name :user-agent
                                                :value :misa/0.1}]
                                     :id :kimi
                                     :max_tokens config.max_tokens
                                     :models (or config.models {})
                                     :models_url models-url
                                     :timeouts config.timeouts
                                     :url (or config.url
                                              (.. profile.api_base :/messages))})
          nil)}

