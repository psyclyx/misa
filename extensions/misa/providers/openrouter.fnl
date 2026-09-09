(local protocol (require :misa.protocols.openai))
(local definitions (require :misa.definitions))

;; OpenRouter API provider declaration.

(fn [context]
  "Describe openrouter policies for the supplied application settings."
  (local declarations [])
  (local providers (or (and (= (type context.config) :table)
                            context.config.providers)
                       nil))
  (var config (or (and (= (type providers) :table) providers.openrouter) nil))
  (set config (or (and (= (type config) :table) config) {}))
  (local models-url (or config.models_url
                        (and (not= config.discover_models false)
                             (= config.models nil)
                             "https://openrouter.ai/api/v1/models")
                        nil))
  (table.insert declarations
                (let [definition {:description "OpenRouter OAuth"
                                  :discover_models (not= models-url nil)
                                  :id :openrouter
                                  :label :OpenRouter
                                  :model_provider :openrouter
                                  :profile {:id :default}
                                  :strategy :loopback_pkce}]
                  {:catalog :auth-providers
                   :id (. definition :id)
                   :value definition}))
  (local configured
         (protocol.configure {:catalogue_authoritative true
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
                                                                  (>= value 0))
                                                         (tset result key
                                                               (* value 1000000))))
                                                     (local request
                                                            (tonumber item.pricing.request))
                                                     (when (and request
                                                                (>= request 0))
                                                       (set result.request
                                                            request))
                                                     (or (and (next result)
                                                              result)
                                                         nil))))
                              :models (or config.models {})
                              :models_credential false
                              :models_url models-url
                              :reasoning_effort (fn [effort]
                                                  "Describe OpenRouter reasoning options."
                                                  {:reasoning {: effort}})
                              :timeouts config.timeouts
                              :url (or config.url
                                       "https://openrouter.ai/api/v1/chat/completions")}))
  (definitions :provider.openrouter declarations configured))
