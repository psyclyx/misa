(local protocol (require :protocol.openai))
(local definitions (require :misa.definitions))

;; OpenAI API provider declaration. API keys remain in the native auth store.

(fn [context]
  "Describe openai policies for the supplied application settings."
  (local declarations [])
  (local providers (or (and (= (type context.config) :table)
                            context.config.providers)
                       nil))
  (var config (or (and (= (type providers) :table) providers.openai) nil))
  (set config (or (and (= (type config) :table) config) {}))
  (local models-url (or config.models_url
                        (and (not= config.discover_models false)
                             (= config.models nil)
                             "https://api.openai.com/v1/models")
                        nil))
  (table.insert declarations
                (let [definition {:description "OpenAI API key"
                                  :discover_models (not= models-url nil)
                                  :id :openai
                                  :label :OpenAI
                                  :model_provider :openai
                                  :strategy :api_key}]
                  {:catalog :auth-providers
                   :id (. definition :id)
                   :value definition}))
  (local configured
         (protocol.configure {:catalogue_authoritative true
                              :credential :openai
                              :id :openai
                              :max_tokens config.max_tokens
                              :model_filter (fn [item]
                                              (or (item.id:match "^gpt%-")
                                                  (item.id:match "^o[134]%-")
                                                  (item.id:match "^o[134]$")))
                              :models (or config.models {})
                              :models_url models-url
                              :timeouts config.timeouts
                              :url (or config.url
                                       "https://api.openai.com/v1/chat/completions")}))
  (definitions :provider.openai declarations configured))
