(fn settings [config]
  "Describe provider transport settings."
  (let [models-url (or config.models_url "https://api.openai.com/v1/models")]
    {:catalogue_authoritative true
     :credential :openai
     :id :openai
     :max_tokens config.max_tokens
     :model_filter (fn [item]
                     (or (item.id:match "^gpt%-") (item.id:match "^o[134]%-")
                         (item.id:match "^o[134]$")))
     :models_url models-url
     :timeouts config.timeouts
     :url (or config.url "https://api.openai.com/v1/chat/completions")}))

{: settings}
