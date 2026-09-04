-- Canonical transcript models and projection. Interactive output is composed
-- only from db.messages.transcript by the root managed view.
local function lower(value) return tostring(value):lower() end
-- Transcript values cross the final trust boundary here. Keep line feeds, turn
-- tabs/CR into stable text, strip terminal controls, and repair malformed UTF-8
-- before any component can project the value into a semantic view.
local function printable_text(value)
  local out, i, length = {}, 1, #value
  local function continuation(byte) return byte and byte >= 0x80 and byte <= 0xbf end
  while i <= length do
    local byte = value:byte(i)
    if byte == 0x1b then
      local next_byte = value:byte(i + 1)
      if next_byte == 0x5b then
        i = i + 2
        while i <= length do local current=value:byte(i); i=i+1; if current>=0x40 and current<=0x7e then break end end
      elseif next_byte == 0x5d then
        i = i + 2
        while i <= length do
          local current = value:byte(i)
          if current == 0x07 then i=i+1; break end
          if current == 0x1b and value:byte(i+1) == 0x5c then i=i+2; break end
          i=i+1
        end
      else i=i+1 end
    elseif byte == 0x09 then out[#out+1]=" "; i=i+1
    elseif byte == 0x0d then out[#out+1]="\n"; i=i+1; if value:byte(i)==0x0a then i=i+1 end
    elseif byte == 0x0a then out[#out+1]="\n"; i=i+1
    elseif byte < 0x20 or byte == 0x7f then i=i+1
    else
      local width = byte < 0x80 and 1 or (byte >= 0xc2 and byte <= 0xdf and 2 or (byte >= 0xe0 and byte <= 0xef and 3 or (byte >= 0xf0 and byte <= 0xf4 and 4 or 0)))
      local b2, b3, b4 = value:byte(i+1), value:byte(i+2), value:byte(i+3)
      local valid = width == 1 or
        (width == 2 and continuation(b2)) or
        (width == 3 and continuation(b2) and continuation(b3) and not (byte==0xe0 and b2<0xa0) and not (byte==0xed and b2>0x9f)) or
        (width == 4 and continuation(b2) and continuation(b3) and continuation(b4) and not (byte==0xf0 and b2<0x90) and not (byte==0xf4 and b2>0x8f))
      if valid then
        if not (byte==0xc2 and b2>=0x80 and b2<=0x9f) then out[#out+1]=value:sub(i,i+width-1) end
        i=i+width
      else out[#out+1]="�"; i=i+1 end
    end
  end
  return table.concat(out)
end
local function truncate_text(value, limit)
  if #value <= limit then return value end
  local boundary = limit
  while boundary > 0 and value:byte(boundary + 1) and value:byte(boundary + 1) >= 0x80 and value:byte(boundary + 1) <= 0xbf do boundary=boundary-1 end
  return value:sub(1,boundary) .. "… [truncated " .. tostring(#value - boundary) .. " bytes]"
end
local function copy_structural(value, policy, depth, key)
  if key and policy.redact[lower(key)] then return "[redacted]" end
  local kind = type(value)
  if kind == "string" then return truncate_text(printable_text(value), policy.max_string) end
  if kind ~= "table" then return value end
  if depth >= policy.max_depth then return "[truncated structure]" end
  local result, count = {}, 0
  for child_key, child in pairs(value) do
    count = count + 1; if count > policy.max_items then result["…"] = "[truncated items]"; break end
    result[child_key] = copy_structural(child, policy, depth + 1, child_key)
  end
  return result
end
local function describe(value, depth)
  local kind = type(value)
  if kind == "string" then return printable_text(value) end
  if kind ~= "table" then return printable_text(tostring(value)) end
  if depth > 3 then return "{…}" end
  local values = {}; for key, child in pairs(value) do values[#values + 1] = printable_text(tostring(key)) .. "=" .. describe(child, depth + 1) end
  table.sort(values); return "{" .. table.concat(values, ", ") .. "}"
end
local function append(db, model)
  local messages = assert(db.messages, "message state is not initialized")
  messages.transcript[#messages.transcript + 1] = model; messages.scroll = 0
end
local function noninteractive_commit(db, role, model, cofx, markdown)
  if cofx.terminal.interactive or not misa.render_component then return {} end
  local rendered = misa.render_component(db, role, model, { interactive = false, columns = cofx.terminal.columns, markdown = markdown })
  if #(rendered.lines or {}) == 0 then return {} end
  return { { type = "view/commit", lines = rendered.lines } }
end

return { setup = function(context)
  local config = type(context.config) == "table" and context.config.messages or nil
  config = type(config) == "table" and config or {}
  local markdown = config.plain ~= true and config.markdown ~= false
  local policy = { max_string = config.max_string or 4000, max_items = config.max_items or 64, max_depth = config.max_depth or 8, redact = {} }
  assert(type(policy.max_string) == "number" and policy.max_string > 0, "messages.max_string must be positive")
  for _, key in ipairs(config.redact_keys or { "authorization", "api_key", "password", "secret", "token" }) do policy.redact[lower(key)] = true end

  misa.reg_keybinding({ context = "global", action = "toggle_verbose", default = { "alt+t" } })
  misa.reg_keybinding({ context = "global", action = "transcript_up", default = { "page_up", "alt+k" } })
  misa.reg_keybinding({ context = "global", action = "transcript_down", default = { "page_down", "alt+j" } })
  misa.reg_command({ name = "/verbose", description = "Toggle thinking and tool transcript detail", event = "messages/toggle-verbose" })
  misa.reg_event("app/start", function(db)
    db.messages = { transcript = {}, verbose = config.verbose == true, scroll = 0 }
    return { db = db }
  end)
  misa.reg_event("messages/toggle-verbose", function(db)
    db.messages.verbose, db.messages.scroll = not db.messages.verbose, 0
    return { db = db, fx = { { type = "dispatch", event = { type = "ui/redraw" } } } }
  end)
  misa.reg_event("messages/scroll", function(db, event)
    db.messages.scroll = math.max(0, db.messages.scroll + event.delta)
    return { db = db, fx = { { type = "terminal/read" } } }
  end)
  misa.reg_interceptor({ id = "messages/global-keys", before = function(tx)
    if tx.event.type == "terminal/input" and not tx.db.picker and misa.keybinding_action then
      local action = misa.keybinding_action("global", tx.event)
      if action == "toggle_verbose" then tx.event = { type = "messages/toggle-verbose" }
      elseif action == "transcript_up" then tx.event = { type = "messages/scroll", delta = math.max(1, math.floor(tx.cofx.terminal.lines / 2)) }
      elseif action == "transcript_down" then tx.event = { type = "messages/scroll", delta = -math.max(1, math.floor(tx.cofx.terminal.lines / 2)) } end
    end
    return tx
  end })
  misa.messages_projection = function(db)
    local state = assert(db.messages, "message state is not initialized")
    return { verbose=state.verbose == true }
  end
  misa.transcript_projection = function(db, render_context)
    local context_copy = {}; for key, value in pairs(render_context or {}) do context_copy[key] = value end
    context_copy.markdown = markdown
    local result, state = {}, assert(db.messages, "message state is not initialized")
    local function project(role, model)
      local projected = model
      if model.kind == "tool_call" then
        projected = {}; for key, value in pairs(model) do projected[key] = value end
        projected.detail = state.verbose and describe(model.arguments, 0) or "collapsed"
      elseif model.kind == "tool_result" then
        projected = {}; for key, value in pairs(model) do projected[key] = value end
        projected.collapsed = not state.verbose
      end
      local rendered = misa.render_component(db, role, projected, context_copy)
      for _, line in ipairs(rendered.lines or {}) do result[#result + 1] = line end
    end
    for _, model in ipairs(state.transcript) do
      if model.kind == "user" then project("transcript.user", model)
      elseif model.kind == "assistant" then project("transcript.assistant", model)
      elseif model.kind == "thinking" then project(state.verbose and "transcript.thinking" or "transcript.thinking_collapsed", model)
      elseif model.kind == "tool_call" then project("transcript.tool_call", model)
      elseif model.kind == "tool_result" then project("transcript.tool_result", model)
      elseif model.kind == "harness" then project("transcript.harness", model) end
    end
    return result
  end
  misa.transcript_window = function(db, render_context, available_lines)
    local lines = misa.transcript_projection(db, render_context)
    local room = math.max(0, math.floor(available_lines or 0)); if room == 0 then return {} end
    local scroll = math.min((db.messages or {}).scroll or 0, math.max(0, #lines - room))
    local first = math.max(1, #lines - room - scroll + 1); local result = {}
    for index=first,math.min(#lines,first+room-1) do result[#result+1]=lines[index] end
    return result
  end
  misa.reg_event("transcript/reset", function(db) db.messages.transcript, db.messages.scroll = {}, 0; return { db = db } end)
  misa.reg_event("transcript/stream-delta", function(db, event)
    assert((event.kind == "text" or event.kind == "thinking") and type(event.text) == "string", "invalid transcript stream delta")
    local kind = event.kind == "text" and "assistant" or "thinking"
    local transcript, model = db.messages.transcript, nil
    local last = transcript[#transcript]
    if last and last.streaming and last.request_id == event.request_id and last.kind == kind then model = last end
    if model then model.text = copy_structural(model.text .. event.text, policy, 0)
    else append(db, { kind = kind, text = copy_structural(event.text, policy, 0), summary = "streaming", streaming = true, request_id = event.request_id }) end
    return { db = db }
  end)
  local function block_model(block, request_id, interrupted)
    if block.type == "text" or block.type == "thinking" then return {
      kind = block.type == "text" and "assistant" or "thinking", text = copy_structural(block.text, policy, 0),
      summary = block.type == "thinking" and "collapsed" or nil, request_id = request_id, interrupted = interrupted or nil,
    } end
    if block.type == "tool_call" then return {
      kind = "tool_call", id = printable_text(tostring(block.id or "")), name = printable_text(tostring(block.name or "")),
      arguments = copy_structural(block.arguments or block.arguments_json or {}, policy, 0), request_id = request_id, interrupted = interrupted or nil,
    } end
  end
  local function replace_request(db, request_id, blocks, interrupted)
    local transcript, retained, insertion = db.messages.transcript, {}, nil
    for _, model in ipairs(transcript) do
      if model.streaming and model.request_id == request_id then insertion = insertion or (#retained + 1)
      else retained[#retained + 1] = model end
    end
    insertion = insertion or (#retained + 1)
    local normalized = {}
    for _, block in ipairs(blocks or {}) do local model = block_model(block, request_id, interrupted); if model then normalized[#normalized + 1] = model end end
    for index = #normalized, 1, -1 do table.insert(retained, insertion, normalized[index]) end
    db.messages.transcript = retained
    return normalized
  end
  misa.reg_event("transcript/interrupted", function(db, event)
    replace_request(db, event.request_id, event.content or {}, true)
    return { db = db }
  end)
  misa.reg_event("transcript/user", function(db, event, cofx)
    local model = { kind = "user", text = copy_structural(event.text, policy, 0) }; append(db, model)
    return { db = db, fx = noninteractive_commit(db, "transcript.user", model, cofx, markdown) }
  end)
  misa.reg_event("transcript/assistant", function(db, event, cofx)
    local models, fx, output = replace_request(db, event.request_id, event.content or {}, false), {}, {}
    for _, model in ipairs(models) do if model.kind == "assistant" then output[#output + 1] = model.text end end
    if #output > 0 then
      local committed = { kind = "assistant", text = table.concat(output, ""), request_id = event.request_id }
      for _, effect in ipairs(noninteractive_commit(db, "transcript.assistant", committed, cofx, markdown)) do fx[#fx + 1] = effect end
    end
    return { db = db, fx = fx }
  end)
  misa.reg_event("transcript/tool-call", function(db, event)
    append(db, { kind = "tool_call", id = printable_text(tostring(event.id or "")), name = printable_text(tostring(event.name or "")), arguments = copy_structural(event.arguments or event.arguments_json or {}, policy, 0) })
    return { db = db }
  end)
  misa.reg_event("transcript/tool-result", function(db, event)
    append(db, { kind = "tool_result", id = event.id, text = copy_structural(tostring(event.text or ""), policy, 0), is_error = event.is_error == true })
    return { db = db }
  end)
  misa.reg_event("transcript/harness", function(db, event, cofx)
    local model = { kind = "harness", text = copy_structural(tostring(event.text or ""), policy, 0), level = event.level }; append(db, model)
    return { db = db, fx = noninteractive_commit(db, "transcript.harness", model, cofx, markdown) }
  end)
end }
