-- Deterministic provider for examples and tests.
local responses
local next_response = 1

return {
  setup = function(context)
    local providers = type(context.config) == "table" and context.config.providers or nil
    local fake = type(providers) == "table" and providers.fake or nil
    responses = type(fake) == "table" and fake.responses or nil
    assert(type(responses) == "table", "config.providers.fake.responses must be an array of strings")
    for index = 1, #responses do
      assert(type(responses[index]) == "string", "config.providers.fake.responses must contain only strings")
    end

    context.misa.register("provider.fake.complete", function(_request)
      local text = responses[next_response]
      assert(type(text) == "string", "config.providers.fake.responses is exhausted")
      next_response = next_response + 1
      return { text = text }
    end)
  end,
}
