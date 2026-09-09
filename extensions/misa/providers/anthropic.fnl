(fn settings [config]
  "Describe provider transport settings."
  (let [models-url (or config.models_url "https://api.anthropic.com/v1/models")]
    {:catalogue_authoritative true
     :credential :anthropic
     :id :anthropic
     :max_tokens config.max_tokens
     :model_filter (fn [item]
                     (not= (item.id:match "^claude%-") nil))
     :models_url models-url
     :thinking config.thinking
     :timeouts config.timeouts
     :url (or config.url "https://api.anthropic.com/v1/messages")}))

{:settings settings}
