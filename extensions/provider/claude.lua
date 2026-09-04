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

local function result_record(records)
  for i = #records, 1, -1 do
    local record = records[i]
    if type(record) == "table" and record.type == "result" then return record end
  end
end

return {
  setup = function(context)
    misa.reg_completion("auth-provider", { value = "claude", label = "Claude", description = "Claude Pro/Max via Claude Code" })
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.claude or nil
    config = type(config) == "table" and config or {}
    local executable = config.executable or "claude"
    assert(type(executable) == "string" and executable ~= "", "config.providers.claude.executable must be nonempty")
    local mcp_command = config.mcp_command or "misa"
    local mcp_arguments = config.mcp_arguments or { "mcp" }
    assert(type(mcp_command) == "string" and mcp_command ~= "", "config.providers.claude.mcp_command must be nonempty")
    assert(type(mcp_arguments) == "table", "config.providers.claude.mcp_arguments must be an array")

    local configured_models = config.models or {
      { id = "claude/opus", model = "opus", label = "Claude Opus", context_window = 200000 },
      { id = "claude/sonnet", model = "sonnet", label = "Claude Sonnet", context_window = 200000 },
      { id = "claude/haiku", model = "haiku", label = "Claude Haiku", context_window = 200000 },
    }
    assert(type(configured_models) == "table" and #configured_models > 0, "config.providers.claude.models must be nonempty")
    for _, model in ipairs(configured_models) do
      assert(type(model) == "table" and type(model.id) == "string" and type(model.model) == "string", "invalid Claude model")
      misa.reg_model({ id = model.id, provider = "claude", model = model.model, label = model.label or model.id, context_window = model.context_window })
    end

    misa.reg_fx("provider.claude", function(effect)
      local argv = {
        executable,
        "--print",
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--verbose",
        "--model", effect.model,
        "--tools", "",
        "--strict-mcp-config",
        "--permission-mode", "dontAsk",
        "--no-session-persistence",
      }
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
        completion = "provider/claude-complete", stdout_format = "json_lines",
      }
    end)

    misa.reg_event("provider/claude-complete", function(_, event)
      if not event.ok then
        return { fx = { { type = "dispatch", event = {
          type = "agent/error", id = event.id,
          message = event.stderr ~= "" and event.stderr or ("claude exited " .. tostring(event.status)),
        } } } }
      end
      local result = result_record(event.records)
      if not result then
        return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = "Claude returned no result record" } } } }
      end
      if result.is_error or type(result.result) ~= "string" then
        local message = type(result.result) == "string" and result.result or "Claude request failed"
        return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = message } } } }
      end
      local usage = type(result.usage) == "table" and result.usage or {}
      return { fx = { { type = "dispatch", event = {
        type = "agent/result", id = event.id, content = { { type = "text", text = result.result } },
        usage = {
          input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
          cache_read_tokens = usage.cache_read_input_tokens or 0, cache_write_tokens = usage.cache_creation_input_tokens or 0,
        },
      } } } }
    end)
  end,
}
