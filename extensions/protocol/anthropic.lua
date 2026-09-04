-- Anthropic Messages protocol adapter shared by provider declarations.
local function message_content(message)
  local blocks = {}
  for _, block in ipairs(message.content or {}) do
    if block.type == "text" then
      blocks[#blocks + 1] = { type = "text", text = block.text }
    elseif block.type == "thinking" then
      -- Thinking signatures are provider-specific and cannot be replayed as
      -- ordinary text; retain visible reasoning only in Misa's history.
    elseif block.type == "tool_call" then
      blocks[#blocks + 1] = { type = "tool_use", id = block.id, name = block.name, input = block.arguments }
    end
  end
  return blocks
end

local function messages(values)
  local result = {}
  for _, message in ipairs(values) do
    if message.role == "user" or message.role == "assistant" then
      result[#result + 1] = { role = message.role, content = message_content(message) }
    elseif message.role == "tool" then
      result[#result + 1] = { role = "user", content = { {
        type = "tool_result", tool_use_id = message.tool_call_id,
        content = message_content(message), is_error = message.is_error == true,
      } } }
    end
  end
  return result
end

local function tools(values)
  local result = {}
  for _, tool in ipairs(values) do
    result[#result + 1] = { name = tool.name, description = tool.description, input_schema = tool.input_schema }
  end
  return result
end

local function response_content(values)
  local result = {}
  for _, block in ipairs(values or {}) do
    if block.type == "text" then
      result[#result + 1] = { type = "text", text = block.text }
    elseif block.type == "thinking" then
      result[#result + 1] = { type = "thinking", text = block.thinking or "" }
    elseif block.type == "tool_use" then
      result[#result + 1] = { type = "tool_call", id = block.id, name = block.name, arguments = block.input }
    end
  end
  return result
end

misa.protocols = misa.protocols or {}
function misa.protocols.anthropic(spec)
  assert(type(spec.id) == "string" and type(spec.url) == "string" and type(spec.models) == "table")
  for _, model in ipairs(spec.models) do
    misa.reg_model({ id = model.id, provider = spec.id, model = model.model, label = model.label or model.id })
  end

  misa.reg_fx("provider." .. spec.id, function(effect)
    local body = {
      model = effect.model,
      max_tokens = spec.max_tokens or 16384,
      messages = messages(effect.messages),
    }
    if effect.system_prompt then body.system = effect.system_prompt end
    local definitions = tools(effect.tools)
    if #definitions > 0 then body.tools = definitions end
    local headers = {
      { name = "content-type", value = "application/json" },
      { name = "anthropic-version", value = "2023-06-01" },
    }
    for _, header in ipairs(spec.headers or {}) do headers[#headers + 1] = header end
    return {
      type = "http/request", method = "POST", url = spec.url, json = body,
      headers = headers,
      credential = { id = spec.credential, header = spec.auth_header or "x-api-key", prefix = spec.auth_prefix or "" },
      response_format = "json", completion = "provider/" .. spec.id .. "-complete", id = effect.id,
    }
  end)

  misa.reg_event("provider/" .. spec.id .. "-complete", function(_, event)
    if not event.ok then
      local detail = event.message or event.body or ("HTTP " .. tostring(event.status))
      return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = detail } } } }
    end
    local data = event.data
    if type(data) ~= "table" or type(data.content) ~= "table" then
      return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = "invalid Anthropic response" } } } }
    end
    local usage = type(data.usage) == "table" and data.usage or {}
    return { fx = { { type = "dispatch", event = {
      type = "agent/result", id = event.id, content = response_content(data.content), stop_reason = data.stop_reason,
      usage = {
        input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
        cache_read_tokens = usage.cache_read_input_tokens or 0, cache_write_tokens = usage.cache_creation_input_tokens or 0,
      },
    } } } }
  end)
end

return { setup = function() end }
