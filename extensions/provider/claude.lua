-- Claude Code process transport. Claude owns subscription credentials; Misa
-- owns conversation policy and invokes the same stream-json protocol used by
-- the Agent SDK.
local function transcript(messages)
  local out = {
    "Continue this conversation. Preserve the roles and treat tool results as authoritative.",
  }
  for _, message in ipairs(messages) do
    if message.role == "user" then
      out[#out + 1] = "\nUser:"
    elseif message.role == "assistant" then
      out[#out + 1] = "\nAssistant:"
    elseif message.role == "tool" then
      out[#out + 1] = "\nTool result for " .. tostring(message.tool_call_id) .. ":"
    end
    for _, block in ipairs(message.content or {}) do
      if block.type == "text" then out[#out + 1] = block.text end
    end
  end
  out[#out + 1] = "\nAssistant:"
  return table.concat(out, "\n")
end

local function json_string(value)
  assert(type(value) == "string" and not value:find("%z"), "MCP command arguments must be strings without NUL")
  return '"' .. value:gsub('[%z\1-\31\\"]', function(char)
    local escapes = { ['"'] = '\\"', ['\\'] = '\\\\', ['\b'] = '\\b', ['\f'] = '\\f', ['\n'] = '\\n', ['\r'] = '\\r', ['\t'] = '\\t' }
    return escapes[char] or string.format("\\u%04x", char:byte())
  end) .. '"'
end

local function mcp_config(command, arguments)
  local encoded = {}
  for _, argument in ipairs(arguments) do encoded[#encoded + 1] = json_string(argument) end
  return '{"mcpServers":{"misa":{"type":"stdio","command":' .. json_string(command) .. ',"args":[' .. table.concat(encoded, ",") .. ']}}}'
end

return {
  setup = function(context)
    misa.reg_auth_provider({ id = "claude", model_provider = "claude", label = "Claude", description = "Claude Pro/Max via Claude Code" })
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.claude or nil
    config = type(config) == "table" and config or {}
    local executable = config.executable or "claude"
    assert(type(executable) == "string" and executable ~= "", "config.providers.claude.executable must be nonempty")
    local mcp_command = config.mcp_command or "misa"
    local mcp_arguments = config.mcp_arguments or { "mcp" }
    assert(type(mcp_command) == "string" and mcp_command ~= "", "config.providers.claude.mcp_command must be nonempty")
    assert(type(mcp_arguments) == "table", "config.providers.claude.mcp_arguments must be an array")

    assert(config.max_plan == nil or type(config.max_plan) == "boolean", "config.providers.claude.max_plan must be boolean")
    local max_plan = config.max_plan == true
    local serializer_id = "claude.cli"
    misa.reg_request_options_serializer(serializer_id, {
      accepts = function(name) return name == "reasoning_effort" end,
      serialize = function(argv, name, value)
        if name ~= "reasoning_effort" then return false end
        argv[#argv + 1] = "--effort"; argv[#argv + 1] = value; return true
      end,
    })
    local reasoning_api = { request_options_serializer = serializer_id, request_options = { reasoning_effort = {
      choices = { "low", "medium", "high", "max" }, default = "high",
    } } }
    local configured_models = config.models or {
      { id = "claude/claude-fable-5-1", model = "claude-fable-5-1", label = "Claude Fable 5.1", context_window = 1000000 },
      { id = "claude/claude-opus-5", model = "claude-opus-5", label = "Claude Opus 5", context_window = max_plan and 1000000 or 200000 },
      { id = "claude/claude-sonnet-5", model = "claude-sonnet-5", label = "Claude Sonnet 5", context_window = 1000000 },
      { id = "claude/claude-haiku-4-5-20251001", model = "claude-haiku-4-5-20251001", label = "Claude Haiku 4.5", context_window = 200000 },
    }
    assert(type(configured_models) == "table" and #configured_models > 0, "config.providers.claude.models must be nonempty")
    for _, model in ipairs(configured_models) do
      assert(type(model) == "table" and type(model.id) == "string" and type(model.model) == "string", "invalid Claude model")
      local api = model.api or reasoning_api
      if type(api) == "table" and type(api.request_options) == "table" then
        local copy = {}; for key, value in pairs(api) do copy[key] = value end; copy.request_options_serializer = serializer_id; api = copy
      end
      misa.reg_model({ id = model.id, provider = "claude", model = model.model, label = model.label or model.id, context_window = model.context_window, api = api })
    end

    if config.max_plan == nil then
      misa.reg_event("models/provider-availability", function(db, event)
        if event.provider ~= "claude" or event.subscription_type == nil or event.subscription_type == misa.json_null then return end
        local has_extended_opus = event.subscription_type == "max" or event.subscription_type == "team" or event.subscription_type == "enterprise"
        local updates = {}
        for _, model in ipairs(configured_models) do
          if model.model:match("^claude%-opus%-") then
            updates[#updates + 1] = { id = model.id, context_window = has_extended_opus and 1000000 or 200000 }
          end
        end
        db.providers = db.providers or {}
        db.providers.claude = db.providers.claude or {}
        db.providers.claude.subscription_type = event.subscription_type
        return { db = db, fx = { { type = "dispatch", event = {
          type = "models/update", provider = "claude", models = updates,
        } } } }
      end)
    end

    misa.reg_fx("provider.claude", function(effect)
      local argv = {
        executable,
        "--print",
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--model", effect.model,
        "--tools", "",
        "--strict-mcp-config",
        "--permission-mode", "dontAsk",
        "--no-session-persistence",
      }
      misa.serialize_request_options(serializer_id, effect.request_options or {}, argv)
      if #effect.tools > 0 then
        local allowed = {}
        for _, tool in ipairs(effect.tools) do allowed[#allowed + 1] = "mcp__misa__" .. tool.name end
        argv[#argv + 1] = "--mcp-config"
        argv[#argv + 1] = mcp_config(mcp_command, mcp_arguments)
        argv[#argv + 1] = "--allowedTools"
        argv[#argv + 1] = table.concat(allowed, ",")
      end
      if effect.system_prompt then argv[#argv + 1] = "--system-prompt"; argv[#argv + 1] = effect.system_prompt end
      return {
        type = "process/run", argv = argv, id = effect.id,
        stdin_json = {
          type = "user", message = { role = "user", content = transcript(effect.messages) },
          parent_tool_use_id = misa.json_null,
        },
        completion = "provider/claude-complete", stdout_format = "json_lines_stream",
      }
    end)

    misa.reg_event("provider/claude-complete", function(db, event)
      db.providers = db.providers or {}; db.providers.claude_streams = db.providers.claude_streams or {}
      if event.phase == "start" then
        db.providers.claude_streams[event.id] = { saw_content = false, saw_stream_event = false, result = false }
        return { db = db, fx = { { type = "dispatch", event = { type = "agent/stream-start", id = event.id } } } }
      end
      local state = db.providers.claude_streams[event.id] or { saw_content = false, saw_stream_event = false, result = false }
      if event.phase == "end" then
        db.providers.claude_streams[event.id] = nil
        local next_event
        if not event.ok then next_event = { type = "agent/stream-error", id = event.id,
          message = event.message or (event.body ~= "" and event.body or ("claude exited " .. tostring(event.status))) }
        elseif not state.result then next_event = { type = "agent/stream-error", id = event.id, message = "Claude returned no result record" }
        else next_event = { type = "agent/stream-end", id = event.id } end
        return { db = db, fx = { { type = "dispatch", event = next_event } } }
      end
      local fx = {}
      for _, record in ipairs(event.records or {}) do
        local partial = record.type == "stream_event" and record.event or nil
        if type(partial) == "table" and (partial.type == "content_block_start" or partial.type == "content_block_delta") then state.saw_stream_event = true; break end
      end
      for _, record in ipairs(event.records or {}) do
        if record.type == "stream_event" and type(record.event) == "table" then
          local partial = record.event
          if partial.type == "content_block_start" and type(partial.content_block) == "table" then
            local block = partial.content_block
            if block.type == "tool_use" then
              state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = {
                type = "tool_call", index = partial.index, id = block.id, name = block.name, arguments_json = "",
              } } }
            elseif block.type == "text" and type(block.text) == "string" and block.text ~= "" then
              state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "text", text = block.text } } }
            elseif block.type == "thinking" and type(block.thinking) == "string" and block.thinking ~= "" then
              state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "thinking", text = block.thinking } } }
            end
          elseif partial.type == "content_block_delta" and type(partial.delta) == "table" then
            local delta = partial.delta
            if delta.type == "text_delta" and type(delta.text) == "string" then state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "text", text = delta.text } } }
            elseif delta.type == "thinking_delta" and type(delta.thinking) == "string" then state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "thinking", text = delta.thinking } } }
            elseif delta.type == "input_json_delta" then state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = {
              type = "tool_call", index = partial.index, arguments_json_delta = delta.partial_json or "",
            } } } end
          elseif partial.type == "message_start" and type(partial.message) == "table" then
            local usage = type(partial.message.usage) == "table" and partial.message.usage or {}
            fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id, usage = {
              input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
              cache_read_tokens = usage.cache_read_input_tokens or 0, cache_write_tokens = usage.cache_creation_input_tokens or 0,
            } } }
          elseif partial.type == "message_delta" then
            local usage = type(partial.usage) == "table" and partial.usage or {}
            fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id,
              stop_reason = type(partial.delta) == "table" and partial.delta.stop_reason or nil, usage = { output_tokens = usage.output_tokens or 0 },
            } }
          end
        elseif record.type == "assistant" and type(record.message) == "table" and not state.saw_stream_event then
          for _, block in ipairs(record.message.content or {}) do
            if block.type == "text" and type(block.text) == "string" then
              state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "text", text = block.text } } }
            elseif block.type == "thinking" and type(block.thinking) == "string" then
              state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "thinking", text = block.thinking } } }
            elseif block.type == "tool_use" then
              state.saw_content = true; fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = {
                type = "tool_call", id = block.id, name = block.name, arguments = block.input,
              } } }
            end
          end
        elseif record.type == "result" then
          state.result = true
          if record.is_error then
            fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-error", id = event.id, message = tostring(record.result or "Claude request failed") } }
          elseif not state.saw_content and type(record.result) == "string" then
            fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "text", text = record.result } } }
          end
          local usage = type(record.usage) == "table" and record.usage or {}
          fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id, usage = {
            input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
            cache_read_tokens = usage.cache_read_input_tokens or 0, cache_write_tokens = usage.cache_creation_input_tokens or 0,
          } } }
        end
      end
      db.providers.claude_streams[event.id] = state
      return { db = db, fx = fx }
    end)
  end,
}
