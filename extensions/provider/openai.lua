-- OpenAI API provider declaration. API keys remain in the native auth store.
return {
  setup = function(context)
    assert(misa.protocols and misa.protocols.openai, "provider.openai requires protocol.openai first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.openai or nil
    config = type(config) == "table" and config or {}
    local models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://api.openai.com/v1/models" or nil)
    misa.reg_auth_provider({ id = "openai", model_provider = "openai", discover_models = models_url ~= nil, label = "OpenAI", description = "OpenAI API key", strategy="api_key" })
    misa.protocols.openai({
      id = "openai",
      credential = "openai",
      url = config.url or "https://api.openai.com/v1/chat/completions",
      timeouts = config.timeouts,
      max_tokens = config.max_tokens,
      models_url = models_url,
      catalogue_authoritative = true,
      model_filter = function(item)
        return item.id:match("^gpt%-") or item.id:match("^o[134]%-") or item.id:match("^o[134]$")
      end,
      models = config.models or {},
    })
  end,
}
