-- OpenRouter API provider declaration.
return {
  setup = function(context)
    misa.reg_completion("auth-provider", { value = "openrouter", label = "OpenRouter", description = "OpenRouter OAuth" })
    assert(misa.protocols and misa.protocols.openai, "provider.openrouter requires protocol.openai first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.openrouter or nil
    config = type(config) == "table" and config or {}
    misa.protocols.openai({
      id = "openrouter",
      credential = "openrouter",
      url = config.url or "https://openrouter.ai/api/v1/chat/completions",
      max_tokens = config.max_tokens,
      models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://openrouter.ai/api/v1/models" or nil),
      models_credential = false,
      headers = {
        { name = "http-referer", value = "https://github.com/psyclyx/misa" },
        { name = "x-title", value = "misa" },
      },
      models = config.models or {
        { id = "openrouter/auto", model = "openrouter/auto", label = "OpenRouter Auto" },
        { id = "openrouter/claude-sonnet-4.6", model = "anthropic/claude-sonnet-4.6", label = "Claude Sonnet 4.6 (OpenRouter)", context_window = 200000 },
        { id = "openrouter/kimi-k2.5", model = "moonshotai/kimi-k2.5", label = "Kimi K2.5 (OpenRouter)", context_window = 262144 },
      },
    })
  end,
}
