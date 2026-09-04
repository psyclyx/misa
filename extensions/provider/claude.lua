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

local function result_record(records)
  for i = #records, 1, -1 do
    local record = records[i]
    if type(record) == "table" and record.type == "result" then return record end
  end
end

return {
  setup = function(context)
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.claude or nil
    config = type(config) == "table" and config or {}
    local executable = config.executable or "claude"
    assert(type(executable) == "string" and executable ~= "", "config.providers.claude.executable must be nonempty")

    local configured_models = config.models or {
      { id = "claude/opus", model = "opus", label = "Claude Opus" },
      { id = "claude/sonnet", model = "sonnet", label = "Claude Sonnet" },
      { id = "claude/haiku", model = "haiku", label = "Claude Haiku" },
    }
    assert(type(configured_models) == "table" and #configured_models > 0, "config.providers.claude.models must be nonempty")
    for _, model in ipairs(configured_models) do
      assert(type(model) == "table" and type(model.id) == "string" and type(model.model) == "string", "invalid Claude model")
      misa.reg_model({ id = model.id, provider = "claude", model = model.model, label = model.label or model.id })
    end

    misa.reg_fx("provider.claude", function(effect)
      local argv = {
        executable,
        "--print",
        "--output-format", "stream-json",
        "--verbose",
        "--model", effect.model,
        "--tools", "",
        "--strict-mcp-config",
        "--permission-mode", "dontAsk",
        "--no-session-persistence",
      }
      if effect.system_prompt then argv[#argv + 1] = "--system-prompt"; argv[#argv + 1] = effect.system_prompt end
      argv[#argv + 1] = "--"
      argv[#argv + 1] = transcript(effect.messages)
      return {
        type = "process/run", argv = argv, id = effect.id,
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
      return { fx = { { type = "dispatch", event = {
        type = "agent/result", id = event.id, content = { { type = "text", text = result.result } },
      } } } }
    end)
  end,
}
