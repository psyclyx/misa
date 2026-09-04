-- Deterministic provider. Its cursor lives in its own model namespace.
return {
  setup = function(context)
    local providers = type(context.config) == "table" and context.config.providers or nil
    local fake = type(providers) == "table" and providers.fake or nil
    local responses = type(fake) == "table" and fake.responses or nil
    assert(type(responses) == "table", "config.providers.fake.responses must be an array of strings")
    for i = 1, #responses do assert(type(responses[i]) == "string", "fake responses must be strings") end

    misa.reg_fx("provider.fake", function(effect)
      assert(type(effect.prompt) == "string" and effect.prompt ~= "", "fake prompt must be a nonempty string")
      assert(type(effect.id) == "string" and effect.id ~= "", "fake id must be a nonempty string")
      return { type = "dispatch", event = { type = "provider/fake", id = effect.id, prompt = effect.prompt } }
    end)

    misa.reg_event("provider/fake", function(db, event)
      assert(type(event.prompt) == "string" and event.prompt ~= "", "fake prompt must be a nonempty string")
      assert(type(event.id) == "string" and event.id ~= "", "fake id must be a nonempty string")
      db.providers = db.providers or {}
      local state = db.providers.fake or { next_response = 1 }
      local text = responses[state.next_response]
      state.next_response = state.next_response + 1
      db.providers.fake = state
      if type(text) ~= "string" then
        return { db = db, fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = "fake responses exhausted" } } } }
      end
      return { db = db, fx = { { type = "dispatch", event = { type = "agent/result", id = event.id, text = text } } } }
    end)
  end,
}
