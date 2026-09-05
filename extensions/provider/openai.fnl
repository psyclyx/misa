;; OpenAI API provider declaration. API keys remain in the native auth store.

{:setup (fn [context]
          (assert (and misa.protocols misa.protocols.openai)
                  "provider.openai requires protocol.openai first")
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table) providers.openai)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local models-url (or config.models_url
                                (or (and (and (not= config.discover_models
                                                    false)
                                              (= config.models nil))
                                         "https://api.openai.com/v1/models")
                                    nil)))
          (misa.reg_auth_provider {:description "OpenAI API key"
                                   :discover_models (not= models-url nil)
                                   :id :openai
                                   :label :OpenAI
                                   :model_provider :openai
                                   :strategy :api_key})
          (misa.protocols.openai {:catalogue_authoritative true
                                  :credential :openai
                                  :id :openai
                                  :max_tokens config.max_tokens
                                  :model_filter (fn [item]
                                                  (or (or (item.id:match "^gpt%-")
                                                          (item.id:match "^o[134]%-"))
                                                      (item.id:match "^o[134]$")))
                                  :models (or config.models {})
                                  :models_url models-url
                                  :timeouts config.timeouts
                                  :url (or config.url
                                           "https://api.openai.com/v1/chat/completions")})
          nil)}

