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
    misa.reg_model({ id = model.id, provider = spec.id, model = model.model, label = model.label or model.id, context_window = model.context_window })
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
        response_format = "json", completion = completion, id = "models-" .. spec.id,
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
    misa.reg_event("model/open", function(db, event)
      if type(event.arguments) == "string" and event.arguments:match("%S") then return end
      if db.models and db.models.available[spec.id] == false then return end
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
