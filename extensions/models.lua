-- Model selection policy. Providers own catalogue entries; this extension owns
-- only the selected model and picker transitions.
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
  return 1
end

return {
  setup = function(context)
    misa.reg_command({ name = "/model", description = "Choose the active model", event = "model/open" })
    local configured = type(context.config) == "table" and context.config.models or nil
    local default = type(configured) == "table" and configured.default or nil
    if default ~= nil then assert(type(default) == "string" and default ~= "", "config.models.default must be a nonempty string") end

    misa.reg_interceptor({
      id = "models/initialize",
      before = function(tx)
        if tx.event.type ~= "app/start" or tx.db.models then return tx end
        local entries = {}
        for _, model in ipairs(misa.models()) do entries[#entries + 1] = copy_model(model) end
        assert(#entries > 0, "no models registered")
        local selected = default or entries[1].id
        assert(find(entries, selected), "configured default model is not registered")
        tx.db.models = { entries = entries, selected = selected, picker = false, index = selected_index(entries, selected) }
        return tx
      end,
    })

    misa.reg_event("model/open", function(db)
      local state = db.models
      assert(state and find(state.entries, state.selected), "model state is not initialized")
      state.picker = true
      state.index = selected_index(state.entries, state.selected)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_interceptor({
      id = "models/input",
      before = function(tx)
        if tx.event.type == "terminal/input" and tx.db.models and tx.db.models.picker then
          tx.event = { type = "model/input", kind = tx.event.kind }
        end
        return tx
      end,
    })

    misa.reg_event("model/input", function(db, event)
      local state = db.models
      local entries = state.entries
      if event.kind == "arrow_up" then
        state.index = state.index == 1 and #entries or state.index - 1
      elseif event.kind == "arrow_down" then
        state.index = state.index == #entries and 1 or state.index + 1
      elseif event.kind == "enter" then
        state.selected, state.picker = entries[state.index].id, false
      elseif event.kind == "escape" or event.kind == "ctrl_c" or event.kind == "eof" then
        state.picker = false
      end
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("models/replace-provider", function(db, event)
      assert(type(event.provider) == "string" and event.provider ~= "", "model provider must be nonempty")
      assert(type(event.models) == "table", "models must be an array")
      local state, entries, seen = assert(db.models, "model state is not initialized"), {}, {}
      for _, model in ipairs(state.entries) do
        if model.provider ~= event.provider or model.id == state.selected then
          entries[#entries + 1] = model
          seen[model.id] = true
        end
      end
      for _, model in ipairs(event.models) do
        assert(type(model) == "table" and type(model.id) == "string" and model.id ~= "", "invalid discovered model")
        assert(type(model.model) == "string" and model.model ~= "", "invalid discovered model ID")
        assert(model.context_window == nil or (type(model.context_window) == "number" and model.context_window > 0 and model.context_window % 1 == 0), "invalid context window")
        if seen[model.id] then
          for i, existing in ipairs(entries) do
            if existing.id == model.id then table.remove(entries, i); break end
          end
        end
        seen[model.id] = true
        entries[#entries + 1] = {
          id = model.id, provider = event.provider, model = model.model,
          label = type(model.label) == "string" and model.label or model.id,
          context_window = model.context_window,
        }
      end
      if #entries == 0 then return { db = db } end
      state.entries = entries
      if not find(entries, state.selected) then state.selected = entries[1].id end
      state.index = selected_index(entries, state.selected)
      return { db = db }
    end)

    misa.reg_event("model/select", function(db, event)
      assert(type(event.id) == "string" and find(db.models.entries, event.id), "unknown model")
      db.models.selected = event.id
      db.models.index = selected_index(db.models.entries, event.id)
      db.models.picker = false
      return { db = db }
    end)
  end,
}
