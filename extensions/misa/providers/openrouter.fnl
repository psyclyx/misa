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

(fn settings [config]
  "Describe provider transport settings."
  (let [models-url (or config.models_url "https://openrouter.ai/api/v1/models")]
    {:catalogue_authoritative true
     :credential :openrouter
     :headers [{:name :http-referer :value "https://github.com/psyclyx/misa"}
               {:name :x-title :value :misa}]
     :id :openrouter
     :max_tokens config.max_tokens
     :model_pricing model-pricing
     :models_credential false
     :models_url models-url
     :reasoning_effort (fn [effort]
                         "Describe OpenRouter reasoning options."
                         {:reasoning {: effort}})
     :timeouts config.timeouts
     :url (or config.url "https://openrouter.ai/api/v1/chat/completions")}))

{:settings settings :model-pricing model-pricing}
