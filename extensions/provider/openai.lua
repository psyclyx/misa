-- OpenAI API provider declaration. API keys remain in the native auth store.
return {
  setup = function(context)
    assert(misa.protocols and misa.protocols.openai, "provider.openai requires protocol.openai first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.openai or nil
    config = type(config) == "table" and config or {}
    misa.protocols.openai({
      id = "openai",
      credential = "openai",
      url = config.url or "https://api.openai.com/v1/chat/completions",
      max_tokens = config.max_tokens,
      models = config.models or {
        { id = "openai/gpt-5.4", model = "gpt-5.4", label = "GPT-5.4", context_window = 1000000 },
        { id = "openai/gpt-5.4-mini", model = "gpt-5.4-mini", label = "GPT-5.4 Mini", context_window = 400000 },
      },
    })
  end,
}
