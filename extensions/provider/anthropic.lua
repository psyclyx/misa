-- Anthropic API provider declaration. Secrets are resolved natively by ID.
return {
  setup = function(context)
    assert(misa.protocols and misa.protocols.anthropic, "provider.anthropic requires protocol.anthropic first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.anthropic or nil
    config = type(config) == "table" and config or {}
    local models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://api.anthropic.com/v1/models" or nil)
    misa.reg_auth_provider({ id = "anthropic", model_provider = "anthropic", discover_models = models_url ~= nil, label = "Anthropic", description = "Anthropic API key" })
    misa.protocols.anthropic({
      id = "anthropic",
      credential = "anthropic",
      url = config.url or "https://api.anthropic.com/v1/messages",
      max_tokens = config.max_tokens,
      models_url = models_url,
      catalogue_authoritative = true,
      model_filter = function(item) return item.id:match("^claude%-") ~= nil end,
      models = config.models or {},
    })
  end,
}
