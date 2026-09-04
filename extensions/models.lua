-- Model selection policy. Providers own catalogue entries; this extension owns
-- availability, filtering, selection, and picker transitions.
local function copy_model(model)
  return {
    id = model.id, provider = model.provider, model = model.model,
    label = model.label, context_window = model.context_window,
  }
end

local function find(entries, id)
  for _, model in ipairs(entries) do if model.id == id then return model end end
end

local function selected_index(entries, selected)
  for i, model in ipairs(entries) do if model.id == selected then return i end end
  return #entries > 0 and 1 or 0
end

local function pop_utf8(value)
  local index = #value
  while index > 0 and value:byte(index) >= 0x80 and value:byte(index) < 0xc0 do index = index - 1 end
  return value:sub(1, math.max(0, index - 1))
end

local function rebuild(state, preferred)
  local entries = {}
  for _, model in ipairs(state.catalogue) do
    if state.available[model.provider] ~= false then entries[#entries + 1] = model end
  end
  state.entries = entries
  if not find(entries, state.selected) then
    state.selected = find(entries, preferred) and preferred or (preferred == nil and #entries > 0 and entries[1].id or nil)
  end

  local query = state.query:lower()
  local filtered = {}
  for _, model in ipairs(entries) do
    local text = (model.id .. " " .. (model.label or "") .. " " .. model.provider):lower()
    if query == "" or text:find(query, 1, true) then filtered[#filtered + 1] = model end
  end
  state.filtered = filtered
  state.index = selected_index(filtered, state.selected)
end

return {
  setup = function(context)
    misa.reg_command({
      name = "/model", description = "Choose the active model", event = "model/open",
      complete = function(prefix, db)
        local result = {}
        for _, model in ipairs(db.models and db.models.entries or {}) do
          if model.id:sub(1, #prefix) == prefix then
            result[#result + 1] = { value = model.id, label = model.id, description = model.label or "" }
          end
        end
        return result
      end,
    })
    local configured = type(context.config) == "table" and context.config.models or nil
    local default = type(configured) == "table" and configured.default or nil
    if default ~= nil then assert(type(default) == "string" and default ~= "", "config.models.default must be a nonempty string") end

    misa.reg_interceptor({
      id = "models/initialize",
      before = function(tx)
        if tx.event.type ~= "app/start" or tx.db.models then return tx end
        local catalogue = {}
        for _, model in ipairs(misa.models()) do catalogue[#catalogue + 1] = copy_model(model) end
        local state = {
          catalogue = catalogue, available = tx.db.provider_availability or {}, entries = {}, filtered = {},
          selected = default, configured_default = default, picker = false, query = "", index = 0,
        }
        rebuild(state, default)
        tx.db.models = state
        return tx
      end,
    })

    misa.reg_event("model/open", function(db, event)
      local state = assert(db.models, "model state is not initialized")
      local requested = type(event.arguments) == "string" and event.arguments:match("^%s*(%S+)%s*$") or nil
      if requested then
        assert(find(state.entries, requested), "unknown or unavailable model")
        state.selected, state.picker = requested, false
        rebuild(state, default)
        return { db = db, fx = { { type = "terminal/read" } } }
      end
      state.picker, state.query = true, ""
      rebuild(state, default)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_interceptor({
      id = "models/input",
      before = function(tx)
        if tx.event.type == "terminal/input" and tx.db.models and tx.db.models.picker then
          tx.event = { type = "model/input", kind = tx.event.kind, text = tx.event.text }
        end
        return tx
      end,
    })

    misa.reg_event("model/input", function(db, event)
      local state = db.models
      local entries = state.filtered
      if event.kind == "text" and type(event.text) == "string" then
        state.query = state.query .. event.text
        rebuild(state, default)
      elseif event.kind == "backspace" then
        state.query = pop_utf8(state.query)
        rebuild(state, default)
      elseif event.kind == "arrow_up" and #entries > 0 then
        state.index = state.index <= 1 and #entries or state.index - 1
      elseif event.kind == "arrow_down" and #entries > 0 then
        state.index = state.index >= #entries and 1 or state.index + 1
      elseif event.kind == "enter" and #entries > 0 then
        state.selected, state.picker = entries[state.index].id, false
        rebuild(state, default)
      elseif event.kind == "escape" or event.kind == "ctrl_c" or event.kind == "ctrl_d" or event.kind == "eof" then
        state.picker = false
      end
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("models/provider-availability", function(db, event)
      assert(type(event.provider) == "string" and type(event.available) == "boolean", "invalid provider availability")
      local state = assert(db.models, "model state is not initialized")
      state.available[event.provider] = event.available
      rebuild(state, default)
      return { db = db }
    end)

    misa.reg_event("models/replace-provider", function(db, event)
      assert(type(event.provider) == "string" and event.provider ~= "", "model provider must be nonempty")
      assert(type(event.models) == "table", "models must be an array")
      local state, catalogue, seen = assert(db.models, "model state is not initialized"), {}, {}
      for _, model in ipairs(state.catalogue) do
        if model.provider ~= event.provider or (model.id == state.selected and event.authoritative ~= true) then
          catalogue[#catalogue + 1] = model
          seen[model.id] = true
        end
      end
      for _, model in ipairs(event.models) do
        assert(type(model) == "table" and type(model.id) == "string" and model.id ~= "", "invalid discovered model")
        assert(type(model.model) == "string" and model.model ~= "", "invalid discovered model ID")
        assert(model.context_window == nil or (type(model.context_window) == "number" and model.context_window > 0 and model.context_window % 1 == 0), "invalid context window")
        if seen[model.id] then
          for i, existing in ipairs(catalogue) do if existing.id == model.id then table.remove(catalogue, i); break end end
        end
        seen[model.id] = true
        catalogue[#catalogue + 1] = {
          id = model.id, provider = event.provider, model = model.model,
          label = type(model.label) == "string" and model.label or model.id,
          context_window = model.context_window,
        }
      end
      state.catalogue = catalogue
      rebuild(state, default)
      return { db = db }
    end)

    misa.reg_event("model/select", function(db, event)
      assert(type(event.id) == "string" and find(db.models.entries, event.id), "unknown or unavailable model")
      db.models.selected, db.models.picker = event.id, false
      rebuild(db.models, default)
      return { db = db }
    end)
  end,
}
