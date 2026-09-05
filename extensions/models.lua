-- Model selection policy. Providers own catalogue entries; this extension owns
-- availability and selection. Search and picker transitions belong to picker.
local function copy_model(model)
  return {
    id = model.id, provider = model.provider, model = model.model,
    label = model.label, context_window = model.context_window, api = model.api,
  }
end

local function find(entries, id)
  for _, model in ipairs(entries) do if model.id == id then return model end end
end

local function select_model(state, id)
  state.selected = id
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
end

return {
  setup = function(context)
    if misa.reg_indicator then misa.reg_indicator({
      id="model", label="model", icon="◆",
      value=function(db) local model=misa.selected_model_projection and misa.selected_model_projection(db); return model and model.id or "none" end,
    }) end
    misa.reg_command({
      name = "/model", description = "Choose the active model", event = "model/open",
      preference_scope = "models", choice_purpose = "models",
      selected = function(db) return db.models and db.models.selected or nil end,
      complete = function(_, db)
        local result = {}
        for _, model in ipairs(db.models and db.models.entries or {}) do
          local label = model.label
          if label == nil or label == "" then label = model.id end
          result[#result + 1] = { value=model.id, label=label, search={model.id,model.label or "",model.model or "",model.provider or ""} }
        end
        return result
      end,
    })
    misa.selected_model_projection = function(db)
      local state = db.models or {}; local model = find(state.entries or {}, state.selected)
      if not model then return nil end
      return { id=model.id, label=model.label, context_window=model.context_window }
    end
    misa.models_projection = function(db)
      local selected = misa.selected_model_projection(db)
      return { selected=selected, configured_default=(db.models or {}).configured_default }
    end

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
          catalogue = catalogue, available = tx.db.provider_availability or {}, entries = {},
          selected = default, configured_default = default,
        }
        rebuild(state, default)
        tx.db.models = state
        return tx
      end,
    })

    misa.reg_event("model/open", function(db, event)
      local state = assert(db.models, "model state is not initialized")
      local requested = type(event.arguments) == "string" and event.arguments:match("^%s*(%S+)%s*$") or nil
      assert(requested and find(state.entries, requested), "unknown or unavailable model")
      select_model(state, requested)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("models/provider-availability", function(db, event)
      assert(type(event.provider) == "string" and type(event.available) == "boolean", "invalid provider availability")
      local state = assert(db.models, "model state is not initialized")
      state.available[event.provider] = event.available
      rebuild(state, default)
      return { db = db }
    end)

    misa.reg_event("models/update", function(db, event)
      assert(type(event.provider) == "string" and type(event.models) == "table", "invalid model update")
      local state = assert(db.models, "model state is not initialized")
      local updates = {}
      for _, update in ipairs(event.models) do
        assert(type(update) == "table" and type(update.id) == "string", "invalid model update")
        assert(update.context_window == nil or (type(update.context_window) == "number" and update.context_window > 0 and update.context_window % 1 == 0), "invalid context window")
        updates[update.id] = update
      end
      for _, model in ipairs(state.catalogue) do
        local update = model.provider == event.provider and updates[model.id] or nil
        if update then
          model.context_window = update.context_window
          if update.api ~= nil then model.api = update.api end
        end
      end
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
          context_window = model.context_window, api = model.api,
        }
      end
      state.catalogue = catalogue
      rebuild(state, default)
      return { db = db }
    end)

    misa.reg_event("model/select", function(db, event)
      assert(type(event.id) == "string" and find(db.models.entries, event.id), "unknown or unavailable model")
      select_model(db.models, event.id)
      return { db = db }
    end)
  end,
}
