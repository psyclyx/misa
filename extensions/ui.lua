-- UTF-8 byte-oriented editor policy and semantic projection. Zig alone owns
-- terminal cell widths and converts cursor.byte within its semantic line.
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

local function normalize_editor(db)
  db.editor = type(db.editor) == "string" and db.editor or ""
  if type(db.editor_cursor) ~= "number" or db.editor_cursor % 1 ~= 0 or db.editor_cursor < 0 or db.editor_cursor > #db.editor then
    db.editor_cursor = #db.editor
  elseif db.editor_cursor < #db.editor then
    local byte = db.editor:byte(db.editor_cursor + 1)
    if byte >= 128 and byte < 192 then db.editor_cursor = #db.editor end
  end
end

local function insert_at_cursor(db, text)
  db.editor = db.editor:sub(1, db.editor_cursor) .. text .. db.editor:sub(db.editor_cursor + 1)
  db.editor_cursor = db.editor_cursor + #text
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
      normalize_editor(db)
      if #cofx.argv == 0 then
        local fx = {}
        -- A stream fallback cannot show the mutable inline frame. Commit one
        -- plain prompt so a misclassified or redirected session is observable
        -- instead of appearing to hang while it waits for input.
        if not cofx.terminal.interactive and plain_prompt then
          fx[#fx + 1] = { type = "view/commit", lines = lines_for("misa> enter a prompt:", "plain") }
        end
        fx[#fx + 1] = { type = "terminal/read" }
        return { db = db, fx = fx }
      end
      return { db = db }
    end)
    misa.reg_event("terminal/input", function(db, event)
      normalize_editor(db)
      if event.kind == "text" then
        insert_at_cursor(db, event.text)
      elseif event.kind == "backspace" then
        local previous = previous_cursor(db.editor, db.editor_cursor)
        db.editor = db.editor:sub(1, previous) .. db.editor:sub(db.editor_cursor + 1)
        db.editor_cursor = previous
      elseif event.kind == "arrow_left" then
        db.editor_cursor = previous_cursor(db.editor, db.editor_cursor)
      elseif event.kind == "arrow_right" then
        db.editor_cursor = next_cursor(db.editor, db.editor_cursor)
      elseif event.kind == "enter" then
        if db.editor ~= "" then
          local prompt = db.editor
          db.editor, db.editor_cursor = "", 0
          return { db = db, fx = { { type = "dispatch", event = { type = "agent/submit", prompt = prompt } } } }
        end
      elseif event.kind == "ctrl_c" or event.kind == "eof" then
        return { db = db, fx = { { type = "app/quit" } } }
      end
      return { db = db, fx = { { type = "terminal/read" } } }
    end)
    -- Completion is decided after all event handlers, so ui works whether its
    -- setup ran before or after agent. The accepted id makes one matching
    -- completion transaction produce exactly one immutable commit.
    misa.reg_interceptor({ id = "ui/completion",
      before = function(tx)
        local event = tx.event
        tx.ui_accepts_completion = (event.type == "agent/result" or event.type == "agent/error")
          and tx.db.status == "working" and tx.db.active_request_id == event.id
        return tx
      end,
      after = function(tx)
        if not tx.ui_accepts_completion then return tx end
        local event = tx.event
        if event.type == "agent/result" and tx.db.status == "done" and tx.db.accepted_request_id == event.id then
          tx.fx[#tx.fx + 1] = { type = "view/commit", lines = lines_for(event.text, "assistant") }
          tx.fx[#tx.fx + 1] = { type = "app/quit" }
        elseif event.type == "agent/error" and tx.db.status == "error" and tx.db.accepted_request_id == event.id then
          tx.fx[#tx.fx + 1] = { type = "view/commit", lines = lines_for(tostring(event.message), "error") }
          tx.fx[#tx.fx + 1] = { type = "app/quit" }
        end
        return tx
      end,
    })
    misa.reg_view(function(db, cofx)
      -- Projection is pure: editor normalization belongs to input events.
      local editor = type(db.editor) == "string" and db.editor or ""
      local editor_cursor = type(db.editor_cursor) == "number" and db.editor_cursor or #editor
      local lines = {}
      if db.status == "working" then lines[#lines + 1] = { spans = { { text = "working…", style = "dim" } } } end
      local cursor = nil
      if db.status ~= "done" and db.status ~= "error" then
        local editor_lines = lines_for(editor, "user", true)
        table.insert(editor_lines[1].spans, 1, { text = "> ", style = "accent" })
        local editor_row, byte = editor_position(editor, editor_cursor)
        local editor_start = #lines + 1
        local row = #lines + editor_row
        for i = 1, #editor_lines do lines[#lines + 1] = editor_lines[i] end
        local clamped_row = math.max(1, math.min(row, #lines, cofx.terminal.lines))
        if clamped_row ~= row then
          byte = clamped_row == editor_start and 2 or 0
        else
          byte = byte + (editor_row == 1 and 2 or 0)
        end
        cursor = { row = clamped_row, byte = byte }
      end
      return { lines = lines, cursor = cursor }
    end)
  end,
}
