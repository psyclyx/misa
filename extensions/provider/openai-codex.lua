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
          result[#result + 1] = { type = "function_call", call_id = block.id, name = block.name, arguments = block.arguments_json or "{}" }
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
    for _, model in ipairs(config.models or {
      { id = "openai-codex/gpt-5.4", model = "gpt-5.4", label = "GPT-5.4 (ChatGPT)", context_window = 1000000 },
      { id = "openai-codex/gpt-5.3-codex", model = "gpt-5.3-codex", label = "GPT-5.3 Codex", context_window = 400000 },
    }) do
      misa.reg_model({ id = model.id, provider = "openai-codex", model = model.model, label = model.label or model.id, context_window = model.context_window })
    end

    misa.reg_fx("provider.openai-codex", function(effect)
      local body = {
        model = effect.model, store = false, stream = true,
        instructions = effect.system_prompt or "You are a helpful coding assistant.",
        input = input(effect.messages), text = { verbosity = "low" },
        include = { "reasoning.encrypted_content" }, tool_choice = "auto", parallel_tool_calls = true,
      }
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
        response_format = "sse_json", completion = "provider/openai-codex-complete", id = effect.id,
      }
    end)

    misa.reg_event("provider/openai-codex-complete", function(_, event)
      if not event.ok then
        return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = event.message or event.body or ("HTTP " .. tostring(event.status)) } } } }
      end
      local text, content, usage = {}, {}, {}
      for _, record in ipairs(event.data or {}) do
        if record.type == "response.output_text.delta" and type(record.delta) == "string" then
          text[#text + 1] = record.delta
        elseif record.type == "response.output_item.done" and type(record.item) == "table" and record.item.type == "function_call" then
          content[#content + 1] = {
            type = "tool_call", id = record.item.call_id, name = record.item.name,
            arguments_json = record.item.arguments or "{}",
          }
        elseif record.type == "response.completed" and type(record.response) == "table" and type(record.response.usage) == "table" then
          usage = record.response.usage
        elseif record.type == "error" then
          return { fx = { { type = "dispatch", event = { type = "agent/error", id = event.id, message = tostring(record.message or "Codex request failed") } } } }
        end
      end
      if #text > 0 then table.insert(content, 1, { type = "text", text = table.concat(text) }) end
      local details = type(usage.input_tokens_details) == "table" and usage.input_tokens_details or {}
      return { fx = { { type = "dispatch", event = {
        type = "agent/result", id = event.id, content = content,
        usage = {
          input_tokens = usage.input_tokens or 0, output_tokens = usage.output_tokens or 0,
          cache_read_tokens = details.cached_tokens or 0, cache_write_tokens = 0,
        },
      } } } }
    end)
  end,
}
