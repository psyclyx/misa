-- Anthropic API provider declaration. Secrets are resolved natively by ID.
return {
  setup = function(context)
    misa.reg_completion("auth-provider", { value = "anthropic", label = "Anthropic", description = "Anthropic API key" })
    assert(misa.protocols and misa.protocols.anthropic, "provider.anthropic requires protocol.anthropic first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.anthropic or nil
    config = type(config) == "table" and config or {}
    local models = config.models
    if models == nil then
      models = {}
      local model_config = type(context.config) == "table" and context.config.models or nil
      local default = type(model_config) == "table" and model_config.default or nil
      local wire_model = type(default) == "string" and default:match("^anthropic/(.+)$") or nil
      if wire_model then models[1] = { id = default, model = wire_model, label = wire_model } end
    end
    misa.protocols.anthropic({
      id = "anthropic",
      credential = "anthropic",
      url = config.url or "https://api.anthropic.com/v1/messages",
      max_tokens = config.max_tokens,
      models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://api.anthropic.com/v1/models" or nil),
      discover_on_start = true,
      catalogue_authoritative = true,
      model_filter = function(item) return item.id:match("^claude%-") ~= nil end,
      models = models,
    })
  end,
}
