(local protocol (require :misa.protocols.anthropic))

(local definitions (require :misa.definitions))

(fn build [context]
  "Build the anthropic provider catalogs from application settings."
  (let [declarations []
        providers (or (and (= (type context.config) :table)
                           context.config.providers) nil)
        raw-config (or (and (= (type providers) :table) providers.anthropic)
                       nil)
        config (or (and (= (type raw-config) :table) raw-config) {})
        models-url (or config.models_url
                       (and (not= config.discover_models false)
                            (= config.models nil)
                            "https://api.anthropic.com/v1/models")
                       nil)]
    (table.insert declarations
                  (let [definition {:description "Anthropic API key"
                                    :discover_models (not= models-url nil)
                                    :id :anthropic
                                    :label :Anthropic
                                    :model_provider :anthropic
                                    :strategy :api_key}]
                    {:catalog :auth-providers
                     :id (. definition :id)
                     :value definition}))
    (let [configured (protocol.configure {:catalogue_authoritative true
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
                                                   "https://api.anthropic.com/v1/messages")})]
      (definitions.build :provider.anthropic declarations configured))))

{:build build}
