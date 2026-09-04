-- OpenAI-compatible Chat Completions protocol adapter.
local function text(content)
  if type(content) == "string" then return content end
  local parts = {}
  for _, block in ipairs(content or {}) do if block.type == "text" then parts[#parts + 1] = block.text end end
  return table.concat(parts, "")
end

local function messages(values)
  local result = {}
  for _, message in ipairs(values) do
    if message.role == "user" then
      result[#result + 1] = { role = "user", content = text(message.content) }
    elseif message.role == "assistant" then
      local item = { role = "assistant", content = text(message.content) }
      local calls = {}
      for _, block in ipairs(message.content or {}) do
        if block.type == "tool_call" then
          calls[#calls + 1] = { id = block.id, type = "function", ["function"] = {
            name = block.name, arguments = block.arguments_json or "{}",
          } }
        end
      end
      if #calls > 0 then item.tool_calls = calls end
      result[#result + 1] = item
    elseif message.role == "tool" then
      result[#result + 1] = { role = "tool", tool_call_id = message.tool_call_id, content = text(message.content) }
    end
  end
  return result
end

local function tools(values)
  local result = {}
  for _, tool in ipairs(values) do
    result[#result + 1] = { type = "function", ["function"] = {
      name = tool.name, description = tool.description, parameters = tool.input_schema,
    } }
  end
  return result
end

misa.protocols = misa.protocols or {}
function misa.protocols.openai(spec)
  assert(type(spec.id) == "string" and type(spec.url) == "string" and type(spec.models) == "table")
  for _, model in ipairs(spec.models) do
    misa.reg_model({ id = model.id, provider = spec.id, model = model.model, label = model.label or model.id })
  end

  misa.reg_fx("provider." .. spec.id, function(effect)
    local converted = messages(effect.messages)
    if effect.system_prompt then table.insert(converted, 1, { role = "system", content = effect.system_prompt }) end
    local body = { model = effect.model, messages = converted }
    local definitions = tools(effect.tools)
    if #definitions > 0 then body.tools = definitions end
    if spec.max_tokens then body.max_completion_tokens = spec.max_tokens end
    local headers = { { name = "content-type", value = "application/json" } }
    for _, header in ipairs(spec.headers or {}) do headers[#headers + 1] = header end
    return {
      type = "http/request", method = "POST", url = spec.url, json = body, headers = headers,
      credential = { id = spec.credential, header = "authorization", prefix = "Bearer " },
      response_format = "json", completion = "provider/" .. spec.id .. "-complete", id = effect.id,
    }
  end)

  misa.reg_event("provider/" .. spec.id .. "-complete", function(_, event)
    if not event.ok then
      local detail = event.message or event.body or ("HTTP " .. tostring(event.status))
      return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = detail } } } }
    end
    local choice = type(event.data) == "table" and type(event.data.choices) == "table" and event.data.choices[1] or nil
    local message = choice and choice.message or nil
    if type(message) ~= "table" then
      return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = "invalid OpenAI response" } } } }
    end
    local content = {}
    if type(message.content) == "string" and message.content ~= "" then content[#content + 1] = { type = "text", text = message.content } end
    for _, call in ipairs(message.tool_calls or {}) do
      local fn = call["function"]
      if type(call.id) == "string" and type(fn) == "table" and type(fn.name) == "string" and type(fn.arguments) == "string" then
        content[#content + 1] = { type = "tool_call", id = call.id, name = fn.name, arguments_json = fn.arguments }
      end
    end
    return { fx = { { type = "dispatch", event = {
      type = "agent/result", id = event.id, content = content, stop_reason = choice.finish_reason,
    } } } }
  end)
end

return { setup = function() end }
