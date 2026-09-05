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
      assert(type(block.arguments) == "table", "canonical tool arguments must be a table")
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

misa.protocols = misa.protocols or {}
misa.protocols.serialize_anthropic_messages = messages
function misa.protocols.anthropic(spec)
  assert(type(spec.id) == "string" and type(spec.url) == "string" and type(spec.models) == "table")
  local serializer_id = "anthropic.messages." .. spec.id
  misa.reg_request_options_serializer(serializer_id, {
    accepts = function(name) return name == "reasoning_effort" end,
    serialize = function(body, name, value)
      if name ~= "reasoning_effort" then return false end
      body.output_config = body.output_config or {}; body.output_config.effort = value
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
    local completion = "provider/" .. spec.id .. "-models"
    local function request(after_id)
      local separator = spec.models_url:find("?", 1, true) and "&" or "?"
      local url = spec.models_url .. separator .. "limit=1000"
      if after_id then
        assert(after_id:match("^[%w._:-]+$"), "invalid Anthropic model cursor")
        url = url .. "&after_id=" .. after_id
      end
      local headers = { { name = "anthropic-version", value = "2023-06-01" } }
      for _, header in ipairs(spec.model_headers or spec.headers or {}) do headers[#headers + 1] = header end
      return {
        type = "http/request", method = "GET", url = url, headers = headers,
        credential = { id = spec.credential, header = spec.auth_header or "x-api-key", prefix = spec.auth_prefix or "" },
        response_format = "json", completion = completion, id = "models-" .. spec.id, timeouts = spec.timeouts,
      }
    end
    local function discover(db)
      db.model_discovery = db.model_discovery or {}
      db.model_discovery[spec.id] = {}
      return { db = db, fx = { request(nil) } }
    end
    misa.reg_event("models/discover", function(db, event)
      if event.provider and event.provider ~= spec.id then return end
      return discover(db)
    end)
    misa.reg_event(completion, function(db, event)
      if not event.ok or type(event.data) ~= "table" or type(event.data.data) ~= "table" then
        if db.model_discovery then db.model_discovery[spec.id] = nil end
        return { db = db, fx = { { type = "dispatch", event = { type = "models/discovery-complete", provider = spec.id } } } }
      end
      db.model_discovery = db.model_discovery or {}
      local discovered = db.model_discovery[spec.id] or {}
      db.model_discovery[spec.id] = discovered
      for _, item in ipairs(event.data.data) do
        if type(item) == "table" and type(item.id) == "string" and (not spec.model_filter or spec.model_filter(item)) then
          discovered[#discovered + 1] = {
            id = spec.id .. "/" .. item.id, model = item.id,
            label = item.display_name or item.name or item.id,
            context_window = item.context_window or item.context_length or (type(item.max_input_tokens) == "number" and item.max_input_tokens > 0 and item.max_input_tokens or nil),
            api = model_api(item.api or (type(item.request_options) == "table" and { request_options = item.request_options } or nil)),
          }
        end
      end
      if event.data.has_more == true then
        local cursor = event.data.last_id
        if type(cursor) ~= "string" or cursor == "" then
          db.model_discovery[spec.id] = nil
          return { db = db, fx = { { type = "dispatch", event = { type = "models/discovery-complete", provider = spec.id } } } }
        end
        return { db = db, fx = { request(cursor) } }
      end
      db.model_discovery[spec.id] = nil
      local effects = { { type = "dispatch", event = { type = "models/discovery-complete", provider = spec.id } } }
      if #discovered > 0 or spec.catalogue_authoritative == true then
        table.sort(discovered, function(left, right) return left.id < right.id end)
        table.insert(effects, 1, { type = "dispatch", event = {
          type = "models/replace-provider", provider = spec.id, models = discovered,
          authoritative = spec.catalogue_authoritative == true,
        } })
      end
      return { db = db, fx = effects }
    end)
  end

  misa.reg_fx("provider." .. spec.id, function(effect)
    local body = {
      model = effect.model,
      max_tokens = spec.max_tokens or 16384,
      messages = messages(effect.messages), stream = true,
    }
    if effect.system_prompt then body.system = effect.system_prompt end
    misa.serialize_request_options(serializer_id, effect.request_options or {}, body)
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
      response_format = "sse_json_stream", completion = "provider/" .. spec.id .. "-complete", id = effect.id, timeouts = spec.timeouts,
    }
  end)

  misa.reg_event("provider/" .. spec.id .. "-complete", function(db, event)
    db.providers = db.providers or {}; db.providers.anthropic_streams = db.providers.anthropic_streams or {}
    local streams = db.providers.anthropic_streams
    if event.phase == "start" then streams[event.id] = false; return { db = db, fx = { { type = "dispatch", event = { type = "agent/stream-start", id = event.id } } } } end
    if event.phase == "end" then
      local terminal = streams[event.id] == true; streams[event.id] = nil
      local body = type(event.body) == "string" and event.body ~= "" and event.body or nil
      local http = type(event.status) == "number" and event.status >= 400 and ("HTTP " .. tostring(event.status) .. (body and (": " .. body) or "")) or nil
      local mismatch=event.message=="CredentialProfileMismatch"
      local next_event = event.ok and terminal and { type = "agent/stream-end", id = event.id } or {
        type = "agent/stream-error", id = event.id, message = mismatch and "credential endpoint profile changed; run /login kimi-coding for the selected region" or http or body or event.message or (not terminal and "Anthropic stream ended without message_stop") or "Anthropic request failed",
      }
      local fx={{type="dispatch",event=next_event}}
      if mismatch then fx[#fx+1]={type="dispatch",event={type="models/provider-availability",provider=spec.id,available=false,reason="relogin-required"}} end
      return { db = db, fx = fx }
    end
    local fx, terminal = {}, false
    for _, record in ipairs(event.records or {}) do
      if record.type == "message_stop" then streams[event.id], terminal = true, true; break
      elseif record.type == "content_block_start" and type(record.content_block) == "table" then
        local block = record.content_block
        if block.type == "tool_use" then fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = {
          type = "tool_call", index = record.index, id = block.id, name = block.name, arguments_json = "",
        } } } end
      elseif record.type == "content_block_delta" and type(record.delta) == "table" then
        local delta = record.delta
        if delta.type == "text_delta" then fx[#fx + 1] = { type = "dispatch", event = {
          type = "agent/stream-delta", id = event.id, delta = { type = "text", text = delta.text or "" },
        } }
        elseif delta.type == "thinking_delta" then fx[#fx + 1] = { type = "dispatch", event = {
          type = "agent/stream-delta", id = event.id, delta = { type = "thinking", text = delta.thinking or "" },
        } }
        elseif delta.type == "input_json_delta" then fx[#fx + 1] = { type = "dispatch", event = {
          type = "agent/stream-delta", id = event.id, delta = { type = "tool_call", index = record.index, arguments_json_delta = delta.partial_json or "" },
        } } end
      elseif record.type == "message_start" and type(record.message) == "table" then
        local usage = type(record.message.usage) == "table" and record.message.usage or {}
        fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id, usage = {
          input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
          cache_read_tokens = usage.cache_read_input_tokens or 0, cache_write_tokens = usage.cache_creation_input_tokens or 0,
        } } }
      elseif record.type == "message_delta" then
        local usage = type(record.usage) == "table" and record.usage or {}
        fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id,
          stop_reason = type(record.delta) == "table" and record.delta.stop_reason or nil,
          usage = { output_tokens = usage.output_tokens or 0 },
        } }
      elseif record.type == "error" then fx[#fx + 1] = { type = "dispatch", event = {
        type = "agent/stream-error", id = event.id, message = tostring(type(record.error) == "table" and record.error.message or "Anthropic request failed"),
      } } end
    end
    if terminal then fx[#fx + 1] = { type = "operation/finish", id = event.id } end
    return { db = db, fx = fx }
  end)
end

return { setup = function() end }
