-- One-shot agent: turn forwarded arguments into one provider request.
local provider_name
local system_prompt

return {
  setup = function(context)
    local agent = type(context.config) == "table" and context.config.agent or nil
    assert(type(agent) == "table", "config.agent must be an object")
    assert(type(agent.provider) == "string" and agent.provider ~= "", "config.agent.provider must be a nonempty string")
    provider_name = agent.provider
    if agent.system_prompt ~= nil then
      assert(type(agent.system_prompt) == "string", "config.agent.system_prompt must be a string")
      system_prompt = agent.system_prompt
    end
  end,

  run = function(context)
    assert(#context.argv > 0, "agent requires forwarded prompt arguments")
    local request = { prompt = table.concat(context.argv, " ") }
    if system_prompt ~= nil then request.system_prompt = system_prompt end

    local handler_name = "provider." .. provider_name .. ".complete"
    assert(context.misa.handler_count(handler_name) == 1, "agent requires exactly one " .. handler_name .. " handler")
    local results = context.misa.call(handler_name, request)
    local response = results[1]
    assert(type(response) == "table" and type(response.text) == "string", "provider response must be { text = string }")
    io.write(response.text)
    if response.text:sub(-1) ~= "\n" then io.write("\n") end
  end,
}
