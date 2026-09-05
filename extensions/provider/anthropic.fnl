;; Anthropic API provider declaration. Secrets are resolved natively by ID.

{:setup (fn [context]
          (assert (and misa.protocols misa.protocols.anthropic)
                  "provider.anthropic requires protocol.anthropic first")
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table) providers.anthropic)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local models-url (or config.models_url
                                (or (and (and (not= config.discover_models
                                                    false)
                                              (= config.models nil))
                                         "https://api.anthropic.com/v1/models")
                                    nil)))
          (misa.reg_auth_provider {:description "Anthropic API key"
                                   :discover_models (not= models-url nil)
                                   :id :anthropic
                                   :label :Anthropic
                                   :model_provider :anthropic
                                   :strategy :api_key})
          (misa.protocols.anthropic {:catalogue_authoritative true
                                     :credential :anthropic
                                     :id :anthropic
                                     :max_tokens config.max_tokens
                                     :model_filter (fn [item]
                                                     (not= (item.id:match "^claude%-")
                                                           nil))
                                     :models (or config.models {})
                                     :models_url models-url
                                     :thinking config.thinking
                                     :timeouts config.timeouts
                                     :url (or config.url
                                              "https://api.anthropic.com/v1/messages")})
          nil)}

