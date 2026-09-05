-- Kimi Code subscription/API provider over Anthropic Messages.
return {
  setup = function(context)
    assert(misa.protocols and misa.protocols.anthropic, "provider.kimi requires protocol.anthropic first")
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.kimi or nil
    config = type(config) == "table" and config or {}
    local profiles={
      global={id="global",authorization_url="https://auth.kimi.ai/api/oauth/device_authorization",token_url="https://auth.kimi.ai/api/oauth/token",api_base="https://api.kimi.ai/coding/v1"},
      mainland={id="mainland",authorization_url="https://auth.kimi.com/api/oauth/device_authorization",token_url="https://auth.kimi.com/api/oauth/token",api_base="https://api.kimi.com/coding/v1"},
    }
    local region=config.region or "global"; local profile=assert(profiles[region],"providers.kimi.region must be global or mainland")
    local models_url = config.models_url or (config.discover_models ~= false and config.models == nil and profile.api_base.."/models" or nil)
    misa.reg_auth_provider({ id = "kimi-coding", model_provider = "kimi", discover_models = models_url ~= nil, label = "Kimi Coding", description = "Kimi coding plan OAuth ("..region..")", strategy="device_oauth", profile=profile })
    misa.protocols.anthropic({
      id = "kimi",
      credential = "kimi-coding",
      auth_header = "authorization",
      auth_prefix = "Bearer ",
      url = config.url or profile.api_base.."/messages",
      timeouts = config.timeouts,
      max_tokens = config.max_tokens,
      models_url = models_url,
      catalogue_authoritative = true,
      headers = { { name = "user-agent", value = "misa/0.1" } },
      models = config.models or {},
    })
  end,
}
