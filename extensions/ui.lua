-- UTF-8 byte-oriented editor policy and semantic projection. Zig owns cells.
local function lines_for(text, style, preserve_trailing)
  local lines = {}
  text = text:gsub("\r\n", "\n"):gsub("\r", "\n")
  if not preserve_trailing and text:sub(-1) == "\n" then text = text:sub(1, -2) end
  if text == "" then return { { spans = { { text = "", style = style } } } } end
  for line in (text .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = { spans = { { text = line, style = style } } } end
  return lines
end

local function previous_cursor(text, cursor)
  if cursor == 0 then return 0 end
  local previous = cursor - 1
  while previous > 0 do
    local byte = text:byte(previous + 1)
    if byte < 128 or byte >= 192 then break end
    previous = previous - 1
  end
  return previous
end

local function next_cursor(text, cursor)
  if cursor >= #text then return #text end
  local next = cursor + 1
  while next < #text do
    local byte = text:byte(next + 1)
    if byte < 128 or byte >= 192 then break end
    next = next + 1
  end
  return next
end

local function editor(db)
  db.ui = db.ui or {}
  local ui = db.ui
  ui.text = type(ui.text) == "string" and ui.text or ""
  if type(ui.cursor) ~= "number" or ui.cursor % 1 ~= 0 or ui.cursor < 0 or ui.cursor > #ui.text then ui.cursor = #ui.text end
  return ui
end

local function labeled_lines(label, text, style)
  local lines = lines_for(text, style)
  table.insert(lines[1].spans, 1, { text = label .. "  ", style = "bold" })
  return lines
end

local function command_matches(prefix)
  local matches = {}
  if prefix:sub(1, 1) ~= "/" or prefix:find("%s") then return matches end
  for _, command in ipairs(misa.commands()) do
    if command.name:sub(1, #prefix) == prefix then matches[#matches + 1] = command end
  end
  return matches
end

local function command_input(text)
  local name, arguments = text:match("^(%S+)%s*(.-)%s*$")
  return name and misa.command(name), arguments
end

local function token_count(agent)
  local usage = agent.usage or {}
  return (usage.input_tokens or 0) + (usage.output_tokens or 0)
end

local function assistant_text(blocks)
  local parts = {}
  for _, block in ipairs(blocks or {}) do if block.type == "text" then parts[#parts + 1] = block.text end end
  return table.concat(parts, "")
end

local function editor_position(text, cursor)
  local prefix = text:sub(1, cursor)
  local row, line_start = 1, 0
  for index in prefix:gmatch("()\n") do row, line_start = row + 1, index end
  return row, cursor - line_start
end

return {
  setup = function(context)
    local config = type(context.config) == "table" and context.config.ui or nil
    local plain_prompt = type(config) == "table" and config.plain_prompt == true

    misa.reg_event("app/start", function(db, _, cofx)
      editor(db)
      if #cofx.argv ~= 0 then return { db = db } end
      local fx = {}
      if not cofx.terminal.interactive and plain_prompt then
        fx[#fx + 1] = { type = "view/commit", lines = lines_for("misa> enter a prompt:", "plain") }
      end
      fx[#fx + 1] = { type = "terminal/read" }
      return { db = db, fx = fx }
    end)

    misa.reg_event("terminal/input", function(db, event, cofx)
      local ui = editor(db)
      if event.kind == "text" then
        ui.text = ui.text:sub(1, ui.cursor) .. event.text .. ui.text:sub(ui.cursor + 1)
        ui.cursor = ui.cursor + #event.text
        ui.completion_prefix, ui.completion_index = nil, nil
      elseif event.kind == "backspace" then
        local previous = previous_cursor(ui.text, ui.cursor)
        ui.text = ui.text:sub(1, previous) .. ui.text:sub(ui.cursor + 1)
        ui.cursor = previous
        ui.completion_prefix, ui.completion_index = nil, nil
      elseif event.kind == "tab" then
        local prefix = ui.completion_prefix or ui.text
        local matches = command_matches(prefix)
        if #matches > 0 then
          ui.completion_index = ui.completion_index and (ui.completion_index % #matches + 1) or 1
          ui.completion_prefix = prefix
          ui.text, ui.cursor = matches[ui.completion_index].name, #matches[ui.completion_index].name
        end
      elseif (event.kind == "arrow_up" or event.kind == "arrow_down") and ui.completion_prefix then
        local matches = command_matches(ui.completion_prefix)
        if #matches > 0 then
          local delta = event.kind == "arrow_up" and -1 or 1
          ui.completion_index = ((ui.completion_index or 1) - 1 + delta) % #matches + 1
          ui.text, ui.cursor = matches[ui.completion_index].name, #matches[ui.completion_index].name
        end
      elseif event.kind == "arrow_left" then
        ui.completion_prefix, ui.completion_index = nil, nil
        ui.cursor = previous_cursor(ui.text, ui.cursor)
      elseif event.kind == "arrow_right" then
        ui.completion_prefix, ui.completion_index = nil, nil
        ui.cursor = next_cursor(ui.text, ui.cursor)
      elseif event.kind == "escape" and ui.completion_prefix then
        ui.completion_prefix, ui.completion_index = nil, nil
      elseif event.kind == "enter" and ui.text ~= "" then
        local prompt = ui.text
        ui.text, ui.cursor, ui.completion_prefix, ui.completion_index = "", 0, nil, nil
        local command, arguments = command_input(prompt)
        if command then
          return { db = db, fx = { { type = "dispatch", event = {
            type = command.event, command = command.name, arguments = arguments,
          } } } }
        end
        local effects = {}
        if cofx.terminal.interactive then effects[#effects + 1] = { type = "view/commit", lines = labeled_lines("You", prompt, "user") } end
        effects[#effects + 1] = { type = "dispatch", event = { type = "agent/submit", prompt = prompt } }
        return { db = db, fx = effects }
      elseif event.kind == "ctrl_c" or event.kind == "eof" then
        return { db = db, fx = { { type = "app/quit" } } }
      end
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("agent/reset", function(db)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("ui/redraw", function(db)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_interceptor({
      id = "ui/completion",
      before = function(tx)
        local agent, event = tx.db.agent, tx.event
        tx.ui_accepts_completion = agent and (event.type == "agent/result" or event.type == "agent/error")
          and agent.status == "working" and agent.active_request_id == event.id
        return tx
      end,
      after = function(tx)
        if not tx.ui_accepts_completion then return tx end
        local agent, event = tx.db.agent, tx.event
        if event.type == "agent/result" and agent.accepted_request_id == event.id then
          local text = assistant_text(event.content)
          if text ~= "" then
            local rendered = tx.cofx.terminal.interactive and labeled_lines("Assistant", text, "assistant") or lines_for(text, "assistant")
            tx.fx[#tx.fx + 1] = { type = "view/commit", lines = rendered }
          end
        elseif event.type == "agent/error" and agent.accepted_request_id == event.id then
          tx.fx[#tx.fx + 1] = { type = "view/commit", lines = lines_for(tostring(event.message), "error") }
        end
        if agent.status == "ready" then
          if agent.exit_after_response then
            tx.fx[#tx.fx + 1] = { type = "app/quit" }
          elseif tx.cofx.terminal.interactive then
            tx.fx[#tx.fx + 1] = { type = "dispatch", event = { type = "ui/redraw" } }
          else
            tx.fx[#tx.fx + 1] = { type = "terminal/read" }
          end
        end
        return tx
      end,
    })

    misa.reg_view(function(db, cofx)
      local ui = db.ui or { text = "", cursor = 0 }
      local agent = db.agent or {}
      local lines = {
        { spans = { { text = "misa", style = "bold" }, { text = "  coding agent", style = "dim" } } },
      }
      local model_state = db.models or {}
      local selected = nil
      for _, model in ipairs(model_state.entries or {}) do if model.id == model_state.selected then selected = model; break end end
      lines[#lines + 1] = { spans = {
        { text = "model  ", style = "dim" },
        { text = selected and (selected.label or selected.id) or "none", style = "accent" },
      } }
      local last = agent.last_usage or {}
      local context_tokens = (last.input_tokens or 0) + (last.output_tokens or 0)
      local context_window = selected and selected.context_window or nil
      lines[#lines + 1] = { spans = {
        { text = "session  ", style = "dim" }, { text = tostring(token_count(agent)) .. " tokens", style = "plain" },
        { text = "    context  ", style = "dim" },
        { text = context_window and (tostring(context_tokens) .. " / " .. tostring(context_window)) or tostring(context_tokens), style = "plain" },
      } }
      if model_state.picker then
        local entries = model_state.entries or {}
        local room = math.max(1, cofx.terminal.lines - #lines)
        local first = math.max(1, math.min(model_state.index - math.floor(room / 2), #entries - room + 1))
        local last_index = math.min(#entries, first + room - 1)
        for i = first, last_index do
          local model = entries[i]
          local marker = i == model_state.index and "> " or "  "
          lines[#lines + 1] = { spans = {
            { text = marker, style = i == model_state.index and "accent" or "plain" },
            { text = model.label or model.id, style = model.id == model_state.selected and "bold" or "plain" },
            { text = "  " .. model.id, style = "dim" },
          } }
        end
        return { lines = lines, cursor = nil }
      end
      if agent.status == "working" then lines[#lines + 1] = { spans = { { text = "working…", style = "dim" } } } end
      if agent.status == "tools" then lines[#lines + 1] = { spans = { { text = "running tools…", style = "dim" } } } end
      local cursor = nil
      if agent.status == "ready" or agent.status == nil then
        local editor_lines = lines_for(ui.text, "user", true)
        table.insert(editor_lines[1].spans, 1, { text = "> ", style = "accent" })
        local editor_row, byte = editor_position(ui.text, ui.cursor)
        local row = #lines + editor_row
        for i = 1, #editor_lines do lines[#lines + 1] = editor_lines[i] end
        local matches = command_matches(ui.completion_prefix or ui.text)
        for i, command in ipairs(matches) do
          local active = ui.completion_prefix and i == ui.completion_index
          lines[#lines + 1] = { spans = {
            { text = active and "> " or "  ", style = active and "accent" or "plain" },
            { text = command.name, style = "bold" }, { text = "  " .. command.description, style = "dim" },
          } }
        end
        row = math.max(1, math.min(row, #lines, cofx.terminal.lines))
        cursor = { row = row, byte = byte + (editor_row == 1 and 2 or 0) }
      end
      return { lines = lines, cursor = cursor }
    end)
  end,
}
