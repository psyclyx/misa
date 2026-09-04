-- Model selection policy. Providers own catalogue entries; this extension owns
-- availability and selection. Search and picker transitions belong to picker.
local function copy_model(model)
  return {
    id = model.id, provider = model.provider, model = model.model,
    label = model.label, context_window = model.context_window,
  }
end

local function find(entries, id)
  for _, model in ipairs(entries) do if model.id == id then return model end end
end

local function picker_items(state)
  local items = {}
  for _, model in ipairs(state.entries) do
    items[#items + 1] = { value = model.id, label = model.id, description = model.label or "" }
  end
  return items
end

local function picker_updates(state)
  if not state.picker_token then return {} end
  return { { type = "dispatch", event = {
    type = "picker/update", id = "models", token = state.picker_token,
    items = picker_items(state), selected = state.selected or misa.json_null,
  } } }
end

local function select_model(state, id)
  state.selected = id
  return picker_updates(state)
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
      if requested then
        assert(find(state.entries, requested), "unknown or unavailable model")
        local effects = select_model(state, requested)
        effects[#effects + 1] = { type = "terminal/read" }
        return { db = db, fx = effects }
      end
      assert(misa.picker, "model picker requires the picker extension")
      state.picker_sequence = (state.picker_sequence or 0) + 1
      state.picker_token = "models:" .. tostring(state.picker_sequence)
      return { db = db, fx = { { type = "dispatch", event = {
        type = "picker/open", id = "models", token = state.picker_token, title = "model", items = picker_items(state),
        selected = state.selected, completion = "model/picked",
      } } } }
    end)

    misa.reg_event("model/picked", function(db, event)
      if event.picker ~= "models" or event.picker_token ~= db.models.picker_token then return end
      db.models.picker_token = nil
      if event.cancelled ~= true then
        assert(type(event.value) == "string" and find(db.models.entries, event.value), "unknown or unavailable model")
        db.models.selected = event.value
      end
      return { db = db, fx = { { type = "terminal/read" } } }
    end)

    misa.reg_event("models/provider-availability", function(db, event)
      assert(type(event.provider) == "string" and type(event.available) == "boolean", "invalid provider availability")
      local state = assert(db.models, "model state is not initialized")
      state.available[event.provider] = event.available
      rebuild(state, default)
      return { db = db, fx = picker_updates(state) }
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
        if update then model.context_window = update.context_window end
      end
      rebuild(state, default)
      return { db = db, fx = picker_updates(state) }
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
      return { db = db, fx = picker_updates(state) }
    end)

    misa.reg_event("model/select", function(db, event)
      assert(type(event.id) == "string" and find(db.models.entries, event.id), "unknown or unavailable model")
      return { db = db, fx = select_model(db.models, event.id) }
    end)
  end,
}
