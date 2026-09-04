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

    misa.reg_event("terminal/input", function(db, event)
      local ui = editor(db)
      if event.kind == "text" then
        ui.text = ui.text:sub(1, ui.cursor) .. event.text .. ui.text:sub(ui.cursor + 1)
        ui.cursor = ui.cursor + #event.text
      elseif event.kind == "backspace" then
        local previous = previous_cursor(ui.text, ui.cursor)
        ui.text = ui.text:sub(1, previous) .. ui.text:sub(ui.cursor + 1)
        ui.cursor = previous
      elseif event.kind == "arrow_left" then
        ui.cursor = previous_cursor(ui.text, ui.cursor)
      elseif event.kind == "arrow_right" then
        ui.cursor = next_cursor(ui.text, ui.cursor)
      elseif event.kind == "enter" and ui.text ~= "" then
        local prompt = ui.text
        ui.text, ui.cursor = "", 0
        return { db = db, fx = { { type = "dispatch", event = { type = "agent/submit", prompt = prompt } } } }
      elseif event.kind == "ctrl_c" or event.kind == "eof" then
        return { db = db, fx = { { type = "app/quit" } } }
      end
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
        if event.type == "agent/result" and agent.status == "done" and agent.accepted_request_id == event.id then
          tx.fx[#tx.fx + 1] = { type = "view/commit", lines = lines_for(event.text, "assistant") }
          tx.fx[#tx.fx + 1] = { type = "app/quit" }
        elseif event.type == "agent/error" and agent.status == "error" and agent.accepted_request_id == event.id then
          tx.fx[#tx.fx + 1] = { type = "view/commit", lines = lines_for(tostring(event.message), "error") }
          tx.fx[#tx.fx + 1] = { type = "app/quit" }
        end
        return tx
      end,
    })

    misa.reg_view(function(db, cofx)
      local ui = db.ui or { text = "", cursor = 0 }
      local agent = db.agent or {}
      local lines = {}
      if agent.status == "working" then lines[#lines + 1] = { spans = { { text = "working…", style = "dim" } } } end
      local cursor = nil
      if agent.status ~= "done" and agent.status ~= "error" then
        local editor_lines = lines_for(ui.text, "user", true)
        table.insert(editor_lines[1].spans, 1, { text = "> ", style = "accent" })
        local editor_row, byte = editor_position(ui.text, ui.cursor)
        local row = #lines + editor_row
        for i = 1, #editor_lines do lines[#lines + 1] = editor_lines[i] end
        row = math.max(1, math.min(row, #lines, cofx.terminal.lines))
        cursor = { row = row, byte = byte + (editor_row == 1 and 2 or 0) }
      end
      return { lines = lines, cursor = cursor }
    end)
  end,
}
