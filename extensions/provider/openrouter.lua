-- OpenRouter API provider declaration.
return {
  setup = function(context)
    assert(misa.protocols and misa.protocols.openai, "provider.openrouter requires protocol.openai first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.openrouter or nil
    config = type(config) == "table" and config or {}
    local models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://openrouter.ai/api/v1/models" or nil)
    misa.reg_auth_provider({ id = "openrouter", model_provider = "openrouter", discover_models = models_url ~= nil, label = "OpenRouter", description = "OpenRouter OAuth" })
    misa.protocols.openai({
      id = "openrouter",
      credential = "openrouter",
      url = config.url or "https://openrouter.ai/api/v1/chat/completions",
      max_tokens = config.max_tokens,
      models_url = models_url,
      models_credential = false,
      catalogue_authoritative = true,
      headers = {
        { name = "http-referer", value = "https://github.com/psyclyx/misa" },
        { name = "x-title", value = "misa" },
      },
      reasoning_effort = function(body, effort) body.reasoning = { effort = effort } end,
      models = config.models or {},
    })
  end,
}
