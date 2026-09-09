(local protocol (require :misa.protocols.openai))

(local definitions (require :misa.definitions))

(fn model-pricing [item]
  "Normalize OpenRouter prices to dollars per million tokens."
  (if (not= (type item.pricing) :table)
      nil
      (let [result {}
            fields {:cache_read :input_cache_read
                    :cache_write :input_cache_write
                    :input :prompt
                    :output :completion}]
        (each [key field (pairs fields)]
          (let [value (tonumber (. item.pricing field))]
            (when (and value (>= value 0))
              (tset result key (* value 1000000)))))
        (let [request (tonumber item.pricing.request)]
          (when (and request (>= request 0))
            (set result.request request))
          (or (and (next result) result) nil)))))

(fn build [context]
  "Build the openrouter provider catalogs from application settings."
  (let [declarations []
        providers (or (and (= (type context.config) :table)
                           context.config.providers) nil)
        raw-config (or (and (= (type providers) :table) providers.openrouter)
                       nil)
        config (or (and (= (type raw-config) :table) raw-config) {})
        models-url (or config.models_url
                       (and (not= config.discover_models false)
                            (= config.models nil)
                            "https://openrouter.ai/api/v1/models")
                       nil)]
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
    (let [configured (protocol.configure {:catalogue_authoritative true
                                          :credential :openrouter
                                          :headers [{:name :http-referer
                                                     :value "https://github.com/psyclyx/misa"}
                                                    {:name :x-title
                                                     :value :misa}]
                                          :id :openrouter
                                          :max_tokens config.max_tokens
                                          :model_pricing model-pricing
                                          :models (or config.models {})
                                          :models_credential false
                                          :models_url models-url
                                          :reasoning_effort (fn [effort]
                                                              "Describe OpenRouter reasoning options."
                                                              {:reasoning {: effort}})
                                          :timeouts config.timeouts
                                          :url (or config.url
                                                   "https://openrouter.ai/api/v1/chat/completions")})]
      (definitions.build :provider.openrouter declarations configured))))

{:build build :model-pricing model-pricing}
