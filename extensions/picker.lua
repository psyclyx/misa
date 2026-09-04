-- Generic searchable single-selection overlay. Feature plugins provide items and
-- receive selection or cancellation through an ordinary completion event.
local function pop_utf8(value)
  local index = #value
  while index > 0 and value:byte(index) >= 0x80 and value:byte(index) < 0xc0 do index = index - 1 end
  return value:sub(1, math.max(0, index - 1))
end

local function copy_items(values)
  assert(type(values) == "table", "picker items must be an array")
  local items, seen = {}, {}
  for _, item in ipairs(values) do
    assert(type(item) == "table" and type(item.value) == "string" and item.value ~= "", "picker item value must be nonempty")
    assert(item.label == nil or type(item.label) == "string", "picker item label must be a string")
    assert(item.description == nil or type(item.description) == "string", "picker item description must be a string")
    assert(not seen[item.value], "picker item values must be unique")
    seen[item.value] = true
    items[#items + 1] = { value = item.value, label = item.label, description = item.description }
  end
  return items
end

local function filter(state)
  local query, filtered = state.query:lower(), {}
  for _, item in ipairs(state.items) do
    local text = (item.value .. " " .. (item.label or "") .. " " .. (item.description or "")):lower()
    if query == "" or text:find(query, 1, true) then filtered[#filtered + 1] = item end
  end
  state.filtered = filtered
  state.index = #filtered > 0 and 1 or 0
  for i, item in ipairs(filtered) do if item.value == state.selected then state.index = i; break end end
end

return {
  setup = function()
    misa.picker = true
    misa.reg_event("picker/open", function(db, event)
      assert(not db.picker, "a picker is already open")
      assert(type(event.id) == "string" and event.id ~= "", "picker ID must be nonempty")
      assert(type(event.token) == "string" and event.token ~= "", "picker token must be nonempty")
      assert(type(event.title) == "string" and event.title ~= "", "picker title must be nonempty")
      assert(type(event.completion) == "string" and event.completion ~= "", "picker completion must be nonempty")
      db.picker = {
        id = event.id, token = event.token, title = event.title,
        items = copy_items(event.items), filtered = {}, query = "", selected = event.selected,
        completion = event.completion, index = 0,
      }
      filter(db.picker)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("picker/update", function(db, event)
      local state = db.picker
      if not state or event.id ~= state.id or event.token ~= state.token then return end
      state.items = copy_items(event.items)
      if event.selected == misa.json_null then state.selected = nil elseif event.selected ~= nil then state.selected = event.selected end
      filter(state)
      return { db = db }
    end)

    misa.reg_interceptor({
      id = "picker/input",
      before = function(tx)
        if tx.event.type == "terminal/input" and tx.db.picker then
          tx.event = { type = "picker/input", kind = tx.event.kind, text = tx.event.text }
        end
        return tx
      end,
    })

    misa.reg_event("picker/input", function(db, event)
      local state = assert(db.picker, "picker is not open")
      if event.kind == "text" and type(event.text) == "string" then
        state.query = state.query .. event.text
        filter(state)
      elseif event.kind == "backspace" then
        state.query = pop_utf8(state.query)
        filter(state)
      elseif event.kind == "arrow_up" and #state.filtered > 0 then
        state.index = state.index <= 1 and #state.filtered or state.index - 1
      elseif event.kind == "arrow_down" and #state.filtered > 0 then
        state.index = state.index >= #state.filtered and 1 or state.index + 1
      elseif event.kind == "enter" and #state.filtered > 0 then
        local item = state.filtered[state.index]
        db.picker = nil
        return { db = db, fx = { { type = "dispatch", event = {
          type = state.completion, picker = state.id, picker_token = state.token, value = item.value, cancelled = false,
        } } } }
      elseif event.kind == "escape" or event.kind == "ctrl_c" or event.kind == "ctrl_d" or event.kind == "eof" then
        db.picker = nil
        return { db = db, fx = { { type = "dispatch", event = {
          type = state.completion, picker = state.id, picker_token = state.token, value = misa.json_null, cancelled = true,
        } } } }
      end
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_view_layer("picker", function(db, cofx)
      local state = db.picker
      if not state then return end
      local entries, lines = state.filtered, {}
      lines[1] = { spans = {
        { text = state.title .. "> ", style = "accent" }, { text = state.query, style = "plain" },
        { text = "  " .. tostring(#entries) .. "/" .. tostring(#state.items), style = "dim" },
      } }
      local room = math.max(0, (cofx.available_lines or cofx.terminal.lines) - 1)
      if #entries == 0 and room > 0 then
        lines[2] = { spans = { { text = "  no matching choices", style = "dim" } } }
      elseif room > 0 then
        local first = math.max(1, math.min(state.index - math.floor(room / 2), #entries - room + 1))
        local last_index = math.min(#entries, first + room - 1)
        for i = first, last_index do
          local item = entries[i]
          lines[#lines + 1] = { spans = {
            { text = i == state.index and "> " or "  ", style = i == state.index and "accent" or "plain" },
            { text = item.label or item.value, style = item.value == state.selected and "bold" or "plain" },
            { text = item.description and ("  " .. item.description) or "", style = "dim" },
          } }
        end
      end
      return { lines = lines, cursor = { row = 1, byte = #state.title + 2 + #state.query }, exclusive = true }
    end)
  end,
}
