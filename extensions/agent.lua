-- Provider-neutral conversation and tool orchestration.
local function content(value, label)
  assert(type(value) == "table", label .. " content must be an array")
  for _, block in ipairs(value) do
    assert(type(block) == "table" and type(block.type) == "string", label .. " content block is invalid")
    if block.type == "text" or block.type == "thinking" then
      assert(type(block.text) == "string", block.type .. " content must carry text")
    elseif block.type == "tool_call" then
      assert(type(block.id) == "string" and block.id ~= "", "tool call id must be nonempty")
      assert(type(block.name) == "string" and block.name ~= "", "tool call name must be nonempty")
      assert(type(block.arguments) == "table" or type(block.arguments_json) == "string", "tool call arguments are missing")
    else
      error("unsupported assistant content block: " .. block.type)
    end
  end
  return value
end

local function selected_model(db)
  local state = assert(db.models, "model state is not initialized")
  for _, model in ipairs(state.entries or {}) do if model.id == state.selected then return model end end
  error("no model selected")
end

local function request(db)
  local agent = db.agent
  agent.request_seq = agent.request_seq + 1
  local id = "agent-" .. tostring(agent.request_seq)
  agent.active_request_id, agent.status = id, "working"
  local selected = selected_model(db)
  return {
    type = "provider." .. selected.provider,
    id = id,
    model = selected.model,
    messages = agent.messages,
    tools = misa.tools(),
    system_prompt = agent.system_prompt,
  }
end

local function record_usage(agent, usage)
  if usage == nil then return end
  assert(type(usage) == "table", "usage must be a table")
  local normalized = {}
  for _, name in ipairs({ "input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens" }) do
    local value = usage[name] or 0
    assert(type(value) == "number" and value >= 0 and value % 1 == 0, name .. " must be a nonnegative integer")
    normalized[name] = value
    agent.usage[name] = agent.usage[name] + value
  end
  agent.last_usage = normalized
end

local function tool_result(call_id, text, is_error)
  return { role = "tool", tool_call_id = call_id, content = { { type = "text", text = tostring(text) } }, is_error = is_error == true }
end

return {
  setup = function(context)
    misa.reg_command({ name = "/clear", description = "Reset conversation and token usage", event = "agent/reset" })
    local config = type(context.config) == "table" and context.config.agent or nil
    config = type(config) == "table" and config or {}
    assert(config.system_prompt == nil or type(config.system_prompt) == "string", "config.agent.system_prompt must be a string")

    misa.reg_event("app/start", function(db, _, cofx)
      db.agent = {
        messages = {}, request_seq = 0, status = "ready", system_prompt = config.system_prompt,
        exit_after_response = #cofx.argv > 0, pending_tools = {}, pending_tool_count = 0,
        usage = { input_tokens = 0, output_tokens = 0, cache_read_tokens = 0, cache_write_tokens = 0 },
      }
      if #cofx.argv == 0 then return { db = db } end
      return { db = db, fx = { { type = "dispatch", event = { type = "agent/submit", prompt = table.concat(cofx.argv, " ") } } } }
    end)

    misa.reg_event("agent/reset", function(db)
      local agent = assert(db.agent, "agent state is not initialized")
      assert(agent.status == "ready", "agent is busy")
      agent.messages, agent.error, agent.last_usage = {}, nil, nil
      agent.usage = { input_tokens = 0, output_tokens = 0, cache_read_tokens = 0, cache_write_tokens = 0 }
      return { db = db }
    end)

    misa.reg_event("agent/submit", function(db, event)
      assert(type(event.prompt) == "string" and event.prompt ~= "", "agent prompt must be nonempty")
      local agent = assert(db.agent, "agent state is not initialized")
      assert(agent.status == "ready", "agent is busy")
      agent.messages[#agent.messages + 1] = { role = "user", content = { { type = "text", text = event.prompt } } }
      return { db = db, fx = { request(db) } }
    end)

    misa.reg_event("agent/result", function(db, event)
      local agent = db.agent
      if not agent or agent.status ~= "working" or event.id ~= agent.active_request_id then return end
      local blocks = content(event.content, "assistant")
      record_usage(agent, event.usage)
      agent.messages[#agent.messages + 1] = { role = "assistant", content = blocks }
      agent.active_request_id, agent.accepted_request_id = nil, event.id
      local effects, saw_tool = {}, false
      for _, block in ipairs(blocks) do
        if block.type == "tool_call" then
          saw_tool = true
          local tool = misa.tool(block.name)
          if tool then
            assert(not agent.pending_tools[block.id], "duplicate tool call id")
            agent.pending_tools[block.id] = { name = block.name, request_id = event.id }
            agent.pending_tool_count = agent.pending_tool_count + 1
            if block.arguments_json then
              effects[#effects + 1] = {
                type = "json/decode", source = block.arguments_json,
                completion = "agent/tool-arguments", id = block.id,
              }
            else
              effects[#effects + 1] = {
                type = tool.effect, request_id = event.id, tool_call_id = block.id,
                name = block.name, arguments = block.arguments,
              }
            end
          else
            agent.messages[#agent.messages + 1] = tool_result(block.id, "unknown tool: " .. block.name, true)
          end
        end
      end
      if agent.pending_tool_count > 0 then
        agent.status = "tools"
      elseif saw_tool then
        effects[#effects + 1] = request(db)
      else
        agent.status = "ready"
      end
      return { db = db, fx = effects }
    end)

    misa.reg_event("agent/tool-arguments", function(db, event)
      local agent = db.agent
      local pending = agent and agent.pending_tools[event.id]
      assert(pending, "unexpected decoded tool arguments")
      if not event.ok or type(event.data) ~= "table" then
        return { fx = { { type = "dispatch", event = {
          type = "tool/result", tool_call_id = event.id, text = event.message or "invalid tool arguments", is_error = true,
        } } } }
      end
      local tool = assert(misa.tool(pending.name), "tool disappeared")
      return { fx = { {
        type = tool.effect, request_id = pending.request_id, tool_call_id = event.id,
        name = pending.name, arguments = event.data,
      } } }
    end)

    misa.reg_event("tool/result", function(db, event)
      local agent = db.agent
      assert(agent and agent.status == "tools", "no tools are pending")
      assert(type(event.tool_call_id) == "string" and agent.pending_tools[event.tool_call_id], "unexpected tool result")
      agent.pending_tools[event.tool_call_id] = nil
      agent.pending_tool_count = agent.pending_tool_count - 1
      agent.messages[#agent.messages + 1] = tool_result(event.tool_call_id, event.text or "", event.is_error)
      if agent.pending_tool_count == 0 then return { db = db, fx = { request(db) } } end
      return { db = db }
    end)

    misa.reg_event("agent/error", function(db, event)
      local agent = db.agent
      if not agent or agent.status ~= "working" or event.id ~= agent.active_request_id then return end
      agent.error, agent.status, agent.active_request_id = tostring(event.message or "provider failed"), "ready", nil
      agent.accepted_request_id = event.id
      return { db = db }
    end)
  end,
}
