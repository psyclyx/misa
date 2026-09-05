;; OpenRouter API provider declaration.

{:setup (fn [context]
          (assert (and misa.protocols misa.protocols.openai)
                  "provider.openrouter requires protocol.openai first")
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table) providers.openrouter)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local models-url (or config.models_url
                                (or (and (and (not= config.discover_models
                                                    false)
                                              (= config.models nil))
                                         "https://openrouter.ai/api/v1/models")
                                    nil)))
          (misa.reg_auth_provider {:description "OpenRouter OAuth"
                                   :discover_models (not= models-url nil)
                                   :id :openrouter
                                   :label :OpenRouter
                                   :model_provider :openrouter
                                   :profile {:id :default}
                                   :strategy :loopback_pkce})
          (misa.protocols.openai {:catalogue_authoritative true
                                  :credential :openrouter
                                  :headers [{:name :http-referer
                                             :value "https://github.com/psyclyx/misa"}
                                            {:name :x-title :value :misa}]
                                  :id :openrouter
                                  :max_tokens config.max_tokens
                                  :model_pricing (fn [item]
                                                   (if (not= (type item.pricing)
                                                             :table)
                                                       nil
                                                       (do
                                                         (local result {})
                                                         (local fields
                                                                {:cache_read :input_cache_read
                                                                 :cache_write :input_cache_write
                                                                 :input :prompt
                                                                 :output :completion})
                                                         (each [key field (pairs fields)]
                                                           (local value
                                                                  (tonumber (. item.pricing
                                                                               field)))
                                                           (when (and value
                                                                      (>= value
                                                                          0))
                                                             (tset result key
                                                                   (* value
                                                                      1000000))))
                                                         (local request
                                                                (tonumber item.pricing.request))
                                                         (when (and request
                                                                    (>= request
                                                                        0))
                                                           (set result.request
                                                                request))
                                                         (or (and (next result)
                                                                  result)
                                                             nil))))
                                  :models (or config.models {})
                                  :models_credential false
                                  :models_url models-url
                                  :reasoning_effort (fn [body effort]
                                                      (set body.reasoning
                                                           {: effort})
                                                      nil)
                                  :timeouts config.timeouts
                                  :url (or config.url
                                           "https://openrouter.ai/api/v1/chat/completions")})
          nil)}

