(local catalogs {})
(each [_ source (ipairs [:misa.standard.protocols.anthropic
                         :misa.standard.protocols.openai
                         :misa.standard.providers.anthropic
                         :misa.standard.providers.kimi
                         :misa.standard.providers.openai
                         :misa.standard.providers.openrouter
                         :misa.standard.providers.openai-codex
                         :misa.standard.providers.claude
                         :misa.standard.providers.auth])]
  (each [kind entries (pairs (require source))]
    (when (not (. catalogs kind)) (tset catalogs kind {}))
    (each [id value (pairs entries)] (tset catalogs kind id value))))

catalogs
