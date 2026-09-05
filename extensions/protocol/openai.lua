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
            name = block.name, arguments = assert(misa.json, "protocol.openai requires json for tool history").encode(block.arguments),
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
misa.protocols.serialize_openai_messages = messages
function misa.protocols.openai(spec)
  assert(type(spec.id) == "string" and type(spec.url) == "string" and type(spec.models) == "table")
  local serializer_id = "openai.chat." .. spec.id
  misa.reg_request_options_serializer(serializer_id, {
    accepts = function(name) return name == "reasoning_effort" end,
    serialize = function(body, name, value)
      if name ~= "reasoning_effort" then return false end
      if spec.reasoning_effort then spec.reasoning_effort(body, value) else body.reasoning_effort = value end
      return true
    end,
  })
  local function model_api(source)
    if type(source) ~= "table" then return source end
    local api = {}; for key, value in pairs(source) do api[key] = value end
    if type(api.request_options) == "table" then api.request_options_serializer = serializer_id end
    return api
  end
  for _, model in ipairs(spec.models) do
    misa.reg_model({ id = model.id, provider = spec.id, model = model.model, label = model.label or model.id, context_window = model.context_window, api = model_api(model.api) })
  end

  if spec.models_url then
    local function discover()
      local credential = spec.models_credential == false and nil or { id = spec.credential, header = "authorization", prefix = "Bearer " }
      return { fx = { {
        type = "http/request", method = "GET", url = spec.models_url,
        headers = spec.model_headers or {}, credential = credential,
        response_format = "json", completion = "provider/" .. spec.id .. "-models", id = "models-" .. spec.id, timeouts = spec.timeouts,
      } } }
    end
    misa.reg_event("models/discover", function(_, event)
      if event.provider and event.provider ~= spec.id then return end
      return discover()
    end)
    misa.reg_event("provider/" .. spec.id .. "-models", function(_, event)
      if not event.ok or type(event.data) ~= "table" or type(event.data.data) ~= "table" then
        return { fx = { { type = "dispatch", event = { type = "models/discovery-complete", provider = spec.id } } } }
      end
      local discovered = {}
      for _, item in ipairs(event.data.data) do
        if type(item) == "table" and type(item.id) == "string" and (not spec.model_filter or spec.model_filter(item)) then
          discovered[#discovered + 1] = {
            id = spec.id .. "/" .. item.id, model = item.id,
            label = item.name or item.id, context_window = item.context_length or item.context_window,
            api = model_api(item.api or (type(item.request_options) == "table" and { request_options = item.request_options } or nil)),
          }
        end
      end
      table.sort(discovered, function(left, right) return left.id < right.id end)
      return { fx = {
        { type = "dispatch", event = {
          type = "models/replace-provider", provider = spec.id, models = discovered,
          authoritative = spec.catalogue_authoritative == true,
        } },
        { type = "dispatch", event = { type = "models/discovery-complete", provider = spec.id } },
      } }
    end)
  end

  misa.reg_fx("provider." .. spec.id, function(effect)
    local converted = messages(effect.messages)
    if effect.system_prompt then table.insert(converted, 1, { role = "system", content = effect.system_prompt }) end
    local body = { model = effect.model, messages = converted, stream = true, stream_options = { include_usage = true } }
    local definitions = tools(effect.tools)
    if #definitions > 0 then body.tools = definitions end
    if spec.max_tokens then body.max_completion_tokens = spec.max_tokens end
    misa.serialize_request_options(serializer_id, effect.request_options or {}, body)
    local headers = { { name = "content-type", value = "application/json" } }
    for _, header in ipairs(spec.headers or {}) do headers[#headers + 1] = header end
    return {
      type = "http/request", method = "POST", url = spec.url, json = body, headers = headers,
      credential = { id = spec.credential, header = "authorization", prefix = "Bearer " },
      response_format = "sse_json_stream", completion = "provider/" .. spec.id .. "-complete", id = effect.id, timeouts = spec.timeouts,
    }
  end)

  misa.reg_event("provider/" .. spec.id .. "-complete", function(db, event)
    db.providers = db.providers or {}; db.providers.openai_streams = db.providers.openai_streams or {}
    local streams = db.providers.openai_streams
    local fx = {}
    if event.phase == "start" then
      streams[event.id] = false
      return { db = db, fx = { { type = "dispatch", event = { type = "agent/stream-start", id = event.id } } } }
    elseif event.phase == "end" then
      local terminal = streams[event.id] == true; streams[event.id] = nil
      local body = type(event.body) == "string" and event.body ~= "" and event.body or nil
      local http = type(event.status) == "number" and event.status >= 400 and ("HTTP " .. tostring(event.status) .. (body and (": " .. body) or "")) or nil
      local next_event = event.ok and terminal and { type = "agent/stream-end", id = event.id } or {
        type = "agent/stream-error", id = event.id, message = http or body or event.message or (not terminal and "OpenAI stream ended without [DONE]") or "OpenAI request failed",
      }
      return { db = db, fx = { { type = "dispatch", event = next_event } } }
    end
    if event.terminal == true then streams[event.id] = true end
    for _, record in ipairs(event.records or {}) do
      local choice = type(record.choices) == "table" and record.choices[1] or nil
      local delta = choice and choice.delta or nil
      if type(delta) == "table" then
        if type(delta.content) == "string" and delta.content ~= "" then fx[#fx + 1] = { type = "dispatch", event = {
          type = "agent/stream-delta", id = event.id, delta = { type = "text", text = delta.content },
        } } end
        local thinking = delta.reasoning_content or delta.reasoning
        if type(thinking) == "string" and thinking ~= "" then fx[#fx + 1] = { type = "dispatch", event = {
          type = "agent/stream-delta", id = event.id, delta = { type = "thinking", text = thinking },
        } } end
        for _, call in ipairs(delta.tool_calls or {}) do
          local fn = type(call["function"]) == "table" and call["function"] or {}
          fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = {
            type = "tool_call", index = call.index or 0, id = call.id, name = fn.name, arguments_json_delta = fn.arguments,
          } } }
        end
      end
      local raw_usage = type(record.usage) == "table" and record.usage or nil
      if raw_usage or (choice and choice.finish_reason) then
        raw_usage = raw_usage or {}; local details = type(raw_usage.prompt_tokens_details) == "table" and raw_usage.prompt_tokens_details or {}
        fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id,
          stop_reason = choice and choice.finish_reason,
          usage = { input_tokens = raw_usage.prompt_tokens or 0, output_tokens = raw_usage.completion_tokens or 0,
            cache_read_tokens = details.cached_tokens or 0, cache_write_tokens = 0 },
        } }
      end
    end
    if event.terminal == true then fx[#fx + 1] = { type = "operation/finish", id = event.id } end
    return { db = db, fx = fx }
  end)
end

return { setup = function() end }
