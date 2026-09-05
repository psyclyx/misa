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
      assert(type(block.arguments) == "table", "tool call arguments must be a table")
      assert(block.arguments_json == nil, "provider JSON must not enter canonical history")
    else
      error("unsupported assistant content block: " .. block.type)
    end
  end
  return value
end

local function selected_model(db)
  local state = assert(db.models, "model state is not initialized")
  for _, model in ipairs(state.entries or {}) do if model.id == state.selected then return model end end
  return nil
end

local function request(db)
  local agent = db.agent
  local selected = assert(selected_model(db), "selected model became unavailable")
  local options, problem = {}, nil
  if misa.prepare_request_options then options, problem = misa.prepare_request_options(db, selected) end
  if problem then return nil, problem end
  agent.request_seq = agent.request_seq + 1
  local id = "agent-" .. tostring(agent.request_seq)
  agent.active_request_id, agent.status, agent.cancel_requested = id, "working", false
  return {
    type = "provider." .. selected.provider,
    id = id,
    model = selected.model,
    messages = agent.messages,
    tools = misa.tools(),
    system_prompt = agent.system_prompt,
    request_options = options,
  }
end

local function blocked(agent, problem)
  agent.status, agent.active_request_id = "ready", nil
  return {
    { type = "dispatch", event = { type = "transcript/harness", text = problem.message, level = "error", problem = problem } },
    { type = "dispatch", event = { type = "agent/status", status = "ready" } },
    { type = "dispatch", event = { type = "agent/completed", exit = agent.exit_after_response } },
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

local function normalize_tool_arguments(blocks)
  local failures = {}
  for _, block in ipairs(blocks) do
    if block.type == "tool_call" then
      if block.arguments == nil then
        assert(type(block.arguments_json) == "string", "tool call arguments are missing")
        assert(misa.json and type(misa.json.decode) == "function", "JSON extension is required for provider tool calls")
        local ok, decoded = pcall(misa.json.decode, block.arguments_json)
        if ok and type(decoded) == "table" then block.arguments = decoded
        else
          block.arguments = {}
          failures[block.id] = ok and "tool arguments must be a JSON object" or tostring(decoded)
        end
      end
      block.arguments_json = nil
    end
  end
  return failures
end

local function complete_response(db, agent, id, blocks, usage, stop_reason)
  local stream = agent.stream
  local argument_failures = normalize_tool_arguments(blocks)
  blocks = content(blocks, "assistant")
  record_usage(agent, usage)
  agent.messages[#agent.messages + 1] = { role = "assistant", content = blocks }
  agent.active_request_id, agent.accepted_request_id, agent.stream = nil, id, nil
  local effects, saw_tool = {}, false
  if stream then
    for _, block in ipairs(blocks) do effects[#effects + 1] = { type = "dispatch", event = {
      type = "transcript/block-end", response_id = id, block_id = block.transcript_id,
      call_id = block.id, name = block.name, arguments = block.arguments,
    } }; block.transcript_id = nil end
    effects[#effects + 1] = { type = "dispatch", event = { type = "transcript/response-end", response_id = id, usage = usage or stream.usage } }
  else
    effects[#effects + 1] = { type = "dispatch", event = { type = "transcript/assistant", content = blocks, request_id = id } }
  end
  effects[#effects + 1] = { type = "dispatch", event = { type = "agent/usage", usage = agent.usage, last_usage = agent.last_usage } }
  for _, block in ipairs(blocks) do
    if block.type == "tool_call" then
      saw_tool = true
      local tool = misa.tool(block.name)
      if tool then
        assert(not agent.pending_tools[block.id], "duplicate tool call id")
        agent.pending_tools[block.id] = { name = block.name, request_id = id }
        agent.pending_tool_count = agent.pending_tool_count + 1
        if argument_failures[block.id] then
          effects[#effects + 1] = { type = "dispatch", event = {
            type = "tool/result", tool_call_id = block.id, text = argument_failures[block.id], is_error = true,
          } }
        else
          effects[#effects + 1] = { type = tool.effect, request_id = id, tool_call_id = block.id, name = block.name, arguments = block.arguments }
        end
      else
        local message = "unknown tool: " .. block.name
        agent.messages[#agent.messages + 1] = tool_result(block.id, message, true)
        effects[#effects + 1] = { type = "dispatch", event = { type = "transcript/tool-result", id = block.id, text = message, is_error = true } }
      end
    end
  end
  if agent.pending_tool_count > 0 then
    agent.status = "tools"
    effects[#effects + 1] = { type = "dispatch", event = { type = "agent/status", status = "tools" } }
  elseif saw_tool then
    local provider, problem = request(db)
    if problem then for _, effect in ipairs(blocked(agent, problem)) do effects[#effects + 1] = effect end
    else
      effects[#effects + 1] = { type = "dispatch", event = { type = "agent/status", status = "working" } }
      effects[#effects + 1] = provider
    end
  else
    agent.status = "ready"
    effects[#effects + 1] = { type = "dispatch", event = { type = "agent/status", status = "ready" } }
    effects[#effects + 1] = { type = "dispatch", event = { type = "agent/completed", id = id, exit = agent.exit_after_response } }
  end
  return { db = db, fx = effects }
end

local function cancelled(db,agent,id,interrupt_response)
  local response_id=id or agent.active_request_id or agent.accepted_request_id
  agent.error,agent.status,agent.active_request_id,agent.stream,agent.cancel_requested=nil,"ready",nil,nil,false
  agent.pending_tools,agent.pending_tool_count={},0
  local effects={}
  if interrupt_response and response_id then effects[#effects+1]={type="dispatch",event={type="transcript/response-interrupted",response_id=response_id}} end
  effects[#effects+1]={type="dispatch",event={type="transcript/harness",text="Cancelled",level="warning"}}
  effects[#effects+1]={type="dispatch",event={type="agent/status",status="ready"}}
  effects[#effects+1]={type="dispatch",event={type="agent/completed",id=response_id,exit=agent.exit_after_response}}
  return {db=db,fx=effects}
end

local function stream_blocks(stream)
  local blocks = {}
  for _, source in ipairs(stream.blocks) do
    if source.type == "tool_call" and not (type(source.id) == "string" and source.id ~= "" and type(source.name) == "string" and source.name ~= "") then return nil end
    local block = { type=source.type, transcript_id=source.transcript_id }
    if source.type == "text" or source.type == "thinking" then block.text=table.concat(source.chunks)
    else
      block.id,block.name,block.arguments=source.id,source.name,source.arguments
      block.arguments_json=source.arguments_json_chunks and table.concat(source.arguments_json_chunks) or source.arguments_json
    end
    blocks[#blocks + 1] = block
  end
  return blocks
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
      local prompt = table.concat(cofx.argv, " ")
      if db.auth_startup and not db.auth_startup.ready then
        db.agent.startup_prompt = prompt
        return { db = db }
      end
      return { db = db, fx = { { type = "dispatch", event = { type = "agent/submit", prompt = prompt } } } }
    end)

    misa.reg_event("auth/startup-ready", function(db)
      local agent = db.agent
      if not agent or not agent.startup_prompt then return end
      local prompt = agent.startup_prompt
      agent.startup_prompt = nil
      return { db = db, fx = { { type = "dispatch", event = { type = "agent/submit", prompt = prompt } } } }
    end)

    misa.reg_event("agent/cancel-active", function(db)
      local agent = db.agent
      if not agent or agent.cancel_requested then return {db=db} end
      local effects={}
      if agent.status=="working" and agent.active_request_id then
        agent.cancel_requested,agent.status=true,"cancelling"
        effects={{type="dispatch",event={type="agent/status",status="cancelling"}},{type="operation/cancel",id=agent.active_request_id}}
      elseif agent.status=="tools" and agent.pending_tool_count>0 then
        agent.cancel_requested,agent.status=true,"cancelling"
        effects[1]={type="dispatch",event={type="agent/status",status="cancelling"}}
        local ids={}; for id in pairs(agent.pending_tools) do ids[#ids+1]=id end; table.sort(ids)
        for _,id in ipairs(ids) do effects[#effects+1]={type="operation/cancel",id=id} end
      end
      return {db=db,fx=effects}
    end)

    misa.reg_event("agent/reset", function(db)
      local agent = assert(db.agent, "agent state is not initialized")
      assert(agent.status == "ready", "agent is busy")
      agent.messages, agent.error, agent.last_usage = {}, nil, {}
      agent.usage = { input_tokens = 0, output_tokens = 0, cache_read_tokens = 0, cache_write_tokens = 0 }
      return { db = db, fx = {
        { type = "dispatch", event = { type = "transcript/reset" } },
        { type = "dispatch", event = { type = "agent/status", status = "ready", usage = agent.usage, last_usage = agent.last_usage } },
      } }
    end)

    misa.reg_event("agent/submit", function(db, event)
      assert(type(event.prompt) == "string" and event.prompt ~= "", "agent prompt must be nonempty")
      local agent = assert(db.agent, "agent state is not initialized")
      assert(agent.status == "ready", "agent is busy")
      if db.auth_startup and not db.auth_startup.ready then
        agent.startup_prompt = event.prompt
        return { db = db }
      end
      if not selected_model(db) then
        local configured = db.models and db.models.configured_default
        local message = configured and ("configured model is unavailable: " .. configured) or "no available models; log in to a provider"
        local problem = { kind = "request_readiness", code = "missing_model", model = configured, message = message }
        return { db = db, fx = {
          { type = "dispatch", event = { type = "transcript/harness", text = message, level = "error", problem = problem } },
          { type = "dispatch", event = { type = "agent/completed", exit = agent.exit_after_response } },
        } }
      end
      local provider, problem = request(db)
      if problem then return { db = db, fx = blocked(agent, problem) } end
      agent.messages[#agent.messages + 1] = { role = "user", content = { { type = "text", text = event.prompt } } }
      return { db = db, fx = {
        { type = "dispatch", event = { type = "transcript/user", text = event.prompt } },
        { type = "dispatch", event = { type = "agent/status", status = "working" } },
        provider,
      } }
    end)

    -- Providers expose one normalized lifecycle. The agent alone correlates,
    -- assembles, and decides when a response enters conversation history.
    misa.reg_event("agent/stream-start", function(db, event)
      local agent = db.agent
      if not agent or agent.status ~= "working" or event.id ~= agent.active_request_id or agent.stream then return end
      agent.stream = { id = event.id, blocks = {}, tools = {}, block_seq = 0 }
      return { db = db, fx = { { type="dispatch", event={ type="transcript/response-start", response_id=event.id, role="assistant" } } } }
    end)

    misa.reg_event("agent/stream-delta", function(db, event)
      local agent, delta = db.agent, event.delta
      local stream = agent and agent.stream
      if not stream or agent.cancel_requested or event.id ~= agent.active_request_id or stream.id ~= event.id or type(delta) ~= "table" then return end
      local fx = {}
      local function start(block, kind)
        stream.block_seq=stream.block_seq+1; block.transcript_id=event.id.."/"..stream.block_seq; stream.blocks[#stream.blocks+1]=block
        fx[#fx+1]={type="dispatch",event={type="transcript/block-start",response_id=event.id,block_id=block.transcript_id,kind=kind,name=block.name,call_id=block.id}}
      end
      if delta.type == "text" or delta.type == "thinking" then
        assert(type(delta.text) == "string", "stream text delta must be a string")
        if delta.text ~= "" then
          local last = stream.blocks[#stream.blocks]
          if not last or last.type ~= delta.type then last={type=delta.type,chunks={}}; start(last,delta.type=="text" and "assistant" or "thinking") end
          last.chunks[#last.chunks+1]=delta.text
          fx[#fx+1]={type="dispatch",event={type="transcript/block-delta",response_id=event.id,block_id=last.transcript_id,text=delta.text}}
        end
      elseif delta.type == "tool_call" then
        local key = tostring(delta.index or delta.id or (#stream.blocks + 1)); local block=stream.tools[key]
        if not block then block={type="tool_call",id=delta.id,name=delta.name,arguments_json_chunks={}}; stream.tools[key]=block; start(block,"tool_call") end
        if delta.id~=nil then block.id=delta.id end; if delta.name~=nil then block.name=delta.name end
        if delta.arguments~=nil then block.arguments,block.arguments_json_chunks=delta.arguments,nil end
        if delta.arguments_json~=nil then block.arguments_json_chunks={delta.arguments_json} end
        if delta.arguments_json_delta~=nil then block.arguments_json_chunks=block.arguments_json_chunks or {}; block.arguments_json_chunks[#block.arguments_json_chunks+1]=delta.arguments_json_delta end
        fx[#fx+1]={type="dispatch",event={type="transcript/block-delta",response_id=event.id,block_id=block.transcript_id,call_id=delta.id,name=delta.name,arguments=delta.arguments,arguments_json_delta=delta.arguments_json or delta.arguments_json_delta}}
      else error("unsupported agent stream delta: " .. tostring(delta.type)) end
      return { db = db, fx = fx }
    end)

    misa.reg_event("agent/stream-usage", function(db, event)
      local agent = db.agent
      local stream = agent and agent.stream
      if not stream or agent.cancel_requested or event.id ~= agent.active_request_id then return end
      stream.usage = stream.usage or {}
      for _, name in ipairs({ "input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens" }) do
        if type(event.usage) == "table" and event.usage[name] ~= nil then stream.usage[name] = event.usage[name] end
      end
      stream.stop_reason = event.stop_reason or stream.stop_reason
      return { db = db }
    end)

    misa.reg_event("agent/stream-end", function(db, event)
      local agent = db.agent
      local stream = agent and agent.stream
      if not stream or event.id ~= agent.active_request_id or stream.id ~= event.id then return end
      if agent.cancel_requested then return cancelled(db,agent,event.id,true) end
      if agent.status ~= "working" then return end
      local blocks = stream_blocks(stream)
      if not blocks then return { db = db, fx = { { type = "dispatch", event = {
        type = "agent/stream-error", id = event.id, message = "provider ended an incomplete tool call",
      } } } } end
      return complete_response(db, agent, event.id, blocks, event.usage or stream.usage, event.stop_reason or stream.stop_reason)
    end)

    -- Compatibility input is immediately normalized; built-in providers never
    -- use this legacy event.
    misa.reg_event("agent/result", function(db, event)
      local agent = db.agent
      if not agent or event.id ~= agent.active_request_id then return end
      if agent.cancel_requested then return cancelled(db,agent,event.id,agent.stream~=nil) end
      if agent.status ~= "working" then return end
      return complete_response(db, agent, event.id, event.content, event.usage, event.stop_reason)
    end)

    misa.reg_event("tool/result", function(db, event)
      local agent = db.agent
      assert(agent and (agent.status == "tools" or agent.status == "cancelling"), "no tools are pending")
      assert(type(event.tool_call_id) == "string" and agent.pending_tools[event.tool_call_id], "unexpected tool result")
      agent.pending_tools[event.tool_call_id] = nil
      agent.pending_tool_count = agent.pending_tool_count - 1
      if agent.cancel_requested then
        if agent.pending_tool_count==0 then return cancelled(db,agent,agent.accepted_request_id,true) end
        return {db=db}
      end
      agent.messages[#agent.messages + 1] = tool_result(event.tool_call_id, event.text or "", event.is_error)
      local effects = { { type = "dispatch", event = {
        type = "transcript/tool-result", id = event.tool_call_id, text = event.text or "", is_error = event.is_error == true,
      } } }
      if agent.pending_tool_count == 0 then
        local provider, problem = request(db)
        if problem then
          for _, effect in ipairs(blocked(agent, problem)) do effects[#effects + 1] = effect end
        else
          effects[#effects + 1] = { type = "dispatch", event = { type = "agent/status", status = "working" } }
          effects[#effects + 1] = provider
        end
      end
      return { db = db, fx = effects }
    end)

    local function stream_error(db, event)
      local agent = db.agent
      if not agent or event.id ~= agent.active_request_id or (agent.status ~= "working" and agent.status ~= "cancelling") then return end
      if agent.cancel_requested then return cancelled(db,agent,event.id,agent.stream~=nil) end
      local had_stream = agent.stream ~= nil
      agent.error, agent.status, agent.active_request_id, agent.stream = tostring(event.message or "provider failed"), "ready", nil, nil
      agent.accepted_request_id = event.id
      local fx = {}
      if had_stream then fx[#fx + 1] = { type = "dispatch", event = {
        type = "transcript/response-interrupted", response_id = event.id,
      } } end
      fx[#fx + 1] = { type = "dispatch", event = { type = "transcript/harness", text = agent.error, level = "error" } }
      fx[#fx + 1] = { type = "dispatch", event = { type = "agent/status", status = "ready" } }
      fx[#fx + 1] = { type = "dispatch", event = { type = "agent/completed", id = event.id, exit = agent.exit_after_response } }
      return { db = db, fx = fx }
    end
    misa.reg_event("agent/stream-error", stream_error)
    misa.reg_event("agent/error", stream_error)
  end,
}
