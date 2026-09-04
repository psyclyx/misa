-- ChatGPT subscription provider using the Codex Responses SSE protocol.
local function input(messages)
  local result = {}
  for _, message in ipairs(messages) do
    if message.role == "tool" then
      local text = ""; for _, block in ipairs(message.content or {}) do if block.type == "text" then text = text .. block.text end end
      result[#result + 1] = { type = "function_call_output", call_id = message.tool_call_id, output = text }
    else
      local content = {}
      for _, block in ipairs(message.content or {}) do
        if block.type == "text" then
          content[#content + 1] = { type = message.role == "assistant" and "output_text" or "input_text", text = block.text }
        elseif block.type == "tool_call" then
          result[#result + 1] = { type = "function_call", call_id = block.id, name = block.name, arguments = assert(misa.json, "provider.openai-codex requires json for tool history").encode(block.arguments) }
        end
      end
      if #content > 0 then result[#result + 1] = { role = message.role, content = content } end
    end
  end
  return result
end

local function tools(values)
  local result = {}
  for _, tool in ipairs(values) do
    result[#result + 1] = { type = "function", name = tool.name, description = tool.description, parameters = tool.input_schema, strict = false }
  end
  return result
end

return {
  setup = function(context)
    misa.reg_auth_provider({ id = "openai-codex", model_provider = "openai-codex", label = "OpenAI Codex", description = "ChatGPT subscription OAuth" })
    local providers = type(context.config) == "table" and context.config.providers or nil
    local config = type(providers) == "table" and providers.openai_codex or nil
    config = type(config) == "table" and config or {}
    local serializer_id = "openai.responses.codex"
    misa.reg_request_options_serializer(serializer_id, {
      accepts = function(name) return name == "reasoning_effort" end,
      serialize = function(body, name, value)
        if name ~= "reasoning_effort" then return false end
        body.reasoning = { effort = value, summary = "auto" }; return true
      end,
    })
    local reasoning_api = { request_options_serializer = serializer_id, request_options = { reasoning_effort = {
      choices = { "low", "medium", "high", "xhigh" }, default = "medium",
    } } }
    for _, model in ipairs(config.models or {
      { id = "openai-codex/gpt-5.4", model = "gpt-5.4", label = "GPT-5.4 (ChatGPT)", context_window = 1000000, api = reasoning_api },
      { id = "openai-codex/gpt-5.3-codex", model = "gpt-5.3-codex", label = "GPT-5.3 Codex", context_window = 400000, api = reasoning_api },
    }) do
      local api = model.api
      if type(api) == "table" and type(api.request_options) == "table" then local copy = {}; for key, value in pairs(api) do copy[key] = value end; copy.request_options_serializer = serializer_id; api = copy end
      misa.reg_model({ id = model.id, provider = "openai-codex", model = model.model, label = model.label or model.id, context_window = model.context_window, api = api })
    end

    misa.reg_fx("provider.openai-codex", function(effect)
      local body = {
        model = effect.model, store = false, stream = true,
        instructions = effect.system_prompt or "You are a helpful coding assistant.",
        input = input(effect.messages), text = { verbosity = "low" },
        include = { "reasoning.encrypted_content" }, tool_choice = "auto", parallel_tool_calls = true,
      }
      misa.serialize_request_options(serializer_id, effect.request_options or {}, body)
      local definitions = tools(effect.tools); if #definitions > 0 then body.tools = definitions end
      return {
        type = "http/request", method = "POST",
        url = config.url or "https://chatgpt.com/backend-api/codex/responses",
        json = body,
        headers = {
          { name = "content-type", value = "application/json" }, { name = "accept", value = "text/event-stream" },
          { name = "openai-beta", value = "responses=experimental" }, { name = "originator", value = "misa" },
          { name = "user-agent", value = "misa/0.1" },
        },
        credential = {
          id = "openai-codex", header = "authorization", prefix = "Bearer ",
          metadata_field = "account_id", metadata_header = "chatgpt-account-id",
        },
        response_format = "sse_json_stream", completion = "provider/openai-codex-complete", id = effect.id,
      }
    end)

    misa.reg_event("provider/openai-codex-complete", function(db, event)
      db.providers = db.providers or {}; db.providers.codex_streams = db.providers.codex_streams or {}
      local streams = db.providers.codex_streams
      if event.phase == "start" then streams[event.id] = false; return { db = db, fx = { { type = "dispatch", event = { type = "agent/stream-start", id = event.id } } } } end
      if event.phase == "end" then
        local terminal = streams[event.id] == true; streams[event.id] = nil
        local next_event = event.ok and terminal and { type = "agent/stream-end", id = event.id } or {
          type = "agent/stream-error", id = event.id, message = event.message or (not terminal and "Codex stream ended without response.completed") or event.body or ("HTTP " .. tostring(event.status)),
        }
        return { db = db, fx = { { type = "dispatch", event = next_event } } }
      end
      local fx = {}
      for _, record in ipairs(event.records or {}) do
        if record.type == "response.output_text.delta" and type(record.delta) == "string" then
          fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "text", text = record.delta } } }
        elseif (record.type == "response.reasoning_summary_text.delta" or record.type == "response.reasoning_text.delta") and type(record.delta) == "string" then
          fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = { type = "thinking", text = record.delta } } }
        elseif record.type == "response.output_item.done" and type(record.item) == "table" and record.item.type == "function_call" then
          fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-delta", id = event.id, delta = {
            type = "tool_call", index = record.output_index, id = record.item.call_id, name = record.item.name, arguments_json = record.item.arguments or "{}",
          } } }
        elseif record.type == "response.completed" and type(record.response) == "table" then
          streams[event.id] = true
          local usage = type(record.response.usage) == "table" and record.response.usage or {}
          local details = type(usage.input_tokens_details) == "table" and usage.input_tokens_details or {}
          fx[#fx + 1] = { type = "dispatch", event = { type = "agent/stream-usage", id = event.id,
            usage = { input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
              cache_read_tokens = details.cached_tokens or 0, cache_write_tokens = 0 },
          } }
        elseif record.type == "error" then fx[#fx + 1] = { type = "dispatch", event = {
          type = "agent/stream-error", id = event.id, message = tostring(record.message or "Codex request failed"),
        } } end
      end
      return { db = db, fx = fx }
    end)
  end,
}
