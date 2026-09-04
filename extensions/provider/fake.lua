-- Deterministic provider. Its cursor lives in its own model namespace.
return {
  setup = function(context)
    local providers = type(context.config) == "table" and context.config.providers or nil
    local fake = type(providers) == "table" and providers.fake or nil
    local responses = type(fake) == "table" and fake.responses or nil
    assert(type(responses) == "table", "config.providers.fake.responses must be an array of strings")
    for i = 1, #responses do assert(type(responses[i]) == "string" or type(responses[i]) == "table", "fake responses must be strings or content arrays") end

    local serializer_id = "fake.options"
    misa.reg_request_options_serializer(serializer_id, {
      accepts = function() return true end,
      serialize = function(target, name, value) target[name] = value; return true end,
    })
    local configured_models = type(fake.models) == "table" and fake.models or { type(fake.model) == "table" and fake.model or {
      id = "fake/default", model = "default", label = "Fake",
    } }
    for _, model in ipairs(configured_models) do
      local api = model.api
      if type(api) == "table" and type(api.request_options) == "table" then local copy = {}; for key, value in pairs(api) do copy[key] = value end; copy.request_options_serializer = serializer_id; api = copy end
      misa.reg_model({ id = model.id, provider = "fake", model = model.model, label = model.label or model.id, context_window = model.context_window, api = api })
    end

    misa.reg_fx("provider.fake", function(effect)
      assert(type(effect.messages) == "table" and #effect.messages > 0, "fake messages must be nonempty")
      assert(type(effect.id) == "string" and effect.id ~= "", "fake id must be a nonempty string")
      local transported = misa.serialize_request_options(serializer_id, effect.request_options or {}, {})
      if type(fake.expect_request_options) == "table" then
        local count = 0
        for name, value in pairs(fake.expect_request_options) do
          count = count + 1
          assert(transported[name] == value, "unexpected fake request option: " .. name)
        end
        if fake.expect_request_options_exact == true then
          local actual = 0; for _ in pairs(transported) do actual = actual + 1 end
          assert(actual == count, "unexpected additional fake request options")
        end
      end
      return { type = "dispatch", event = { type = "provider/fake", id = effect.id } }
    end)

    misa.reg_event("provider/fake", function(db, event)
      assert(type(event.id) == "string" and event.id ~= "", "fake id must be a nonempty string")
      db.providers = db.providers or {}
      local state = db.providers.fake or { next_response = 1 }
      local response = responses[state.next_response]
      state.next_response = state.next_response + 1
      db.providers.fake = state
      if response == nil then
        return { db = db, fx = { { type = "dispatch", event = { type = "agent/stream-error", id = event.id, message = "fake responses exhausted" } } } }
      end
      local chunks, usage, failure
      if type(response) == "table" and type(response.stream) == "table" then
        chunks, usage, failure = response.stream, response.usage, response.error
      else
        chunks = type(response) == "string" and { { type = "text", text = response } } or response
      end
      local fx = { { type = "dispatch", event = { type = "agent/stream-start", id = event.id } } }
      for index, chunk in ipairs(chunks) do
        assert(type(chunk) == "table" and type(chunk.type) == "string", "fake stream chunks must be normalized deltas")
        local delta = chunk
        if chunk.type == "tool_call" and chunk.index == nil then
          delta = {}; for key, value in pairs(chunk) do delta[key] = value end; delta.index = index
        end
        fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = delta } }
      end
      fx[#fx + 1] = { type = "dispatch", event = failure and
        { type = "agent/stream-error", id = event.id, message = failure } or
        { type = "agent/stream-end", id = event.id, usage = usage } }
      return { db = db, fx = fx }
    end)
  end,
}
