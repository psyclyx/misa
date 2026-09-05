;; Anthropic API provider declaration. Secrets are resolved natively by ID.

{:setup (fn [context]
          (local setup-fx [])
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
          (table.insert setup-fx
                        {:type :register/auth-provider
                         :value {:description "Anthropic API key"
                                 :discover_models (not= models-url nil)
                                 :id :anthropic
                                 :label :Anthropic
                                 :model_provider :anthropic
                                 :strategy :api_key}})
          (each [_ declaration (ipairs (. (misa.protocols.anthropic {:catalogue_authoritative true
                                                                     :credential :anthropic
                                                                     :id :anthropic
                                                                     :max_tokens config.max_tokens
                                                                     :model_filter (fn [item]
                                                                                     (not= (item.id:match "^claude%-")
                                                                                           nil))
                                                                     :models (or config.models
                                                                                 {})
                                                                     :models_url models-url
                                                                     :thinking config.thinking
                                                                     :timeouts config.timeouts
                                                                     :url (or config.url
                                                                              "https://api.anthropic.com/v1/messages")})
                                          :fx))]
            (table.insert setup-fx declaration))
          {:fx setup-fx})}
