-- Kimi Code subscription/API provider over Anthropic Messages.
return {
  setup = function(context)
    misa.reg_completion("auth-provider", { value = "kimi-coding", label = "Kimi Coding", description = "Kimi coding plan OAuth" })
    assert(misa.protocols and misa.protocols.anthropic, "provider.kimi requires protocol.anthropic first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.kimi or nil
    config = type(config) == "table" and config or {}
    misa.protocols.anthropic({
      id = "kimi",
      credential = "kimi-coding",
      auth_header = "authorization",
      auth_prefix = "Bearer ",
      url = config.url or "https://api.kimi.com/coding/v1/messages",
      max_tokens = config.max_tokens,
      models_url = config.models_url or (config.discover_models ~= false and config.models == nil and "https://api.kimi.com/coding/v1/models" or nil),
      headers = { { name = "user-agent", value = "misa/0.1" } },
      models = config.models or {
        { id = "kimi/kimi-for-coding", model = "kimi-for-coding", label = "Kimi For Coding", context_window = 262144 },
      },
    })
  end,
}
