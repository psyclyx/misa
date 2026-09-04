-- Model selection policy. Providers own catalogue entries; this extension owns
-- only the selected model and picker transitions.
local function selected_index(entries, selected)
  for i, model in ipairs(entries) do if model.id == selected then return i end end
  return 1
end

return {
  setup = function(context)
    local configured = type(context.config) == "table" and context.config.models or nil
    local default = type(configured) == "table" and configured.default or nil
    if default ~= nil then assert(type(default) == "string" and default ~= "", "config.models.default must be a nonempty string") end

    misa.reg_interceptor({
      id = "models/initialize",
      before = function(tx)
        if tx.event.type ~= "app/start" or tx.db.models then return tx end
        local entries = misa.models()
        assert(#entries > 0, "no models registered")
        local selected = default or entries[1].id
        assert(misa.model(selected), "configured default model is not registered")
        tx.db.models = { selected = selected, picker = false, index = selected_index(entries, selected) }
        return tx
      end,
    })

    misa.reg_event("model/open", function(db)
      local state = db.models
      assert(state and misa.model(state.selected), "model state is not initialized")
      state.picker = true
      state.index = selected_index(misa.models(), state.selected)
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
      local entries, state = misa.models(), db.models
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

    misa.reg_event("model/select", function(db, event)
      assert(type(event.id) == "string" and misa.model(event.id), "unknown model")
      db.models.selected = event.id
      db.models.index = selected_index(misa.models(), event.id)
      db.models.picker = false
      return { db = db }
    end)
  end,
}
