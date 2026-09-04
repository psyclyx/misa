-- Anthropic API provider declaration. Secrets are resolved natively by ID.
return {
  setup = function(context)
    misa.reg_completion("auth-provider", { value = "anthropic", label = "Anthropic", description = "Anthropic API key" })
    assert(misa.protocols and misa.protocols.anthropic, "provider.anthropic requires protocol.anthropic first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.anthropic or nil
    config = type(config) == "table" and config or {}
    misa.protocols.anthropic({
      id = "anthropic",
      credential = "anthropic",
      url = config.url or "https://api.anthropic.com/v1/messages",
      max_tokens = config.max_tokens,
      models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://api.anthropic.com/v1/models?limit=1000" or nil),
      model_filter = function(item) return item.id:match("^claude%-") ~= nil end,
      models = config.models or {
        { id = "anthropic/claude-fable-5-1", model = "claude-fable-5-1", label = "Claude Fable 5.1", context_window = 1000000 },
        { id = "anthropic/claude-opus-5", model = "claude-opus-5", label = "Claude Opus 5", context_window = 1000000 },
        { id = "anthropic/claude-sonnet-5", model = "claude-sonnet-5", label = "Claude Sonnet 5", context_window = 1000000 },
        { id = "anthropic/claude-haiku-4-5-20251001", model = "claude-haiku-4-5-20251001", label = "Claude Haiku 4.5", context_window = 200000 },
      },
    })
  end,
}
