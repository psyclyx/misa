-- Generic choice preferences backed by the native namespaced state effects.
local function new_state()
  return { clock = 0, scopes = {} }
end

local function state(db)
  db.preferences = db.preferences or new_state()
  return db.preferences
end

local function scope_state(preferences, scope)
  preferences.scopes[scope] = preferences.scopes[scope] or {}
  return preferences.scopes[scope]
end

local function validate(preferences)
  if type(preferences) ~= "table" or type(preferences.clock) ~= "number" or
      preferences.clock < 0 or preferences.clock % 1 ~= 0 or type(preferences.scopes) ~= "table" then return false end
  for scope, values in pairs(preferences.scopes) do
    if type(scope) ~= "string" or scope == "" or type(values) ~= "table" then return false end
    for value, entry in pairs(values) do
      if type(value) ~= "string" or value == "" or type(entry) ~= "table" or
          type(entry.favorite) ~= "boolean" or type(entry.uses) ~= "number" or
          entry.uses < 0 or entry.uses % 1 ~= 0 then return false end
      if entry.last ~= nil and (type(entry.last) ~= "number" or entry.last < 0 or
          entry.last % 1 ~= 0 or entry.last > preferences.clock) then return false end
      if entry.uses > 0 and entry.last == nil then return false end
    end
  end
  return true
end

local function merge_configured_favorites(preferences, configured)
  if configured == nil then return end
  assert(type(configured) == "table", "config.preferences.favorites must be an object")
  for scope, favorites in pairs(configured) do
    assert(type(scope) == "string" and scope ~= "" and type(favorites) == "table", "invalid configured favorites")
    local values = scope_state(preferences, scope)
    for _, value in ipairs(favorites) do
      assert(type(value) == "string" and value ~= "", "configured favorites must be nonempty strings")
      local entry = values[value] or { uses = 0 }
      entry.favorite = true
      values[value] = entry
    end
  end
end

local function copy_item(item)
  return { value = item.value, label = item.label, description = item.description }
end

local function panels(preferences, scope, source)
  local values, favorites, recent, all = scope_state(preferences, scope), {}, {}, {}
  for _, item in ipairs(source) do
    local copy, preference = copy_item(item), values[item.value]
    all[#all + 1] = copy
    if preference and preference.favorite then favorites[#favorites + 1] = copy_item(item) end
    if preference and (preference.uses or 0) > 0 then
      recent[#recent + 1] = { item = copy_item(item), score = (preference.last or 0) * 1000000 + preference.uses }
    end
  end
  table.sort(favorites, function(left, right) return (left.label or left.value) < (right.label or right.value) end)
  table.sort(recent, function(left, right) return left.score > right.score end)
  local result = {}
  if #favorites > 0 then result[#result + 1] = { id = "favorites", title = "Favorites", items = favorites } end
  if #recent > 0 then
    local items = {}; for _, entry in ipairs(recent) do items[#items + 1] = entry.item end
    result[#result + 1] = { id = "recent", title = "Recent", items = items }
  end
  result[#result + 1] = { id = "all", title = "All", items = all }
  return result
end

return {
  setup = function(context)
    local preferences_config = type(context.config) == "table" and context.config.preferences or nil
    if preferences_config ~= nil then assert(type(preferences_config) == "table", "config.preferences must be an object") end
    local configured_favorites = preferences_config and preferences_config.favorites or nil

    misa.reg_event("app/start", function(db)
      return { db = db, fx = { { type = "state/load", namespace = "preferences", completion = "preferences/loaded" } } }
    end)
    misa.reg_event("preferences/loaded", function(db, event)
      assert(event.namespace == "preferences", "invalid preference namespace")
      local preferences = event.found == false and new_state() or event.data
      assert(validate(preferences), "invalid preference data")
      merge_configured_favorites(preferences, configured_favorites)
      db.preferences = preferences
      return { db = db }
    end)

    misa.reg_interceptor({
      id = "preferences/project-picker",
      before = function(tx)
        local event = tx.event
        if event.type == "picker/open" and type(event.preference_scope) == "string" and event.panels == nil then
          event.panels = panels(state(tx.db), event.preference_scope, event.items or {})
        end
        return tx
      end,
    })

    misa.reg_event("choice/used", function(db, event)
      if type(event.scope) ~= "string" or type(event.value) ~= "string" then return end
      local preferences = state(db)
      preferences.clock = preferences.clock + 1
      local values = scope_state(preferences, event.scope)
      local entry = values[event.value] or { uses = 0, favorite = false }
      entry.uses, entry.last = entry.uses + 1, preferences.clock
      values[event.value] = entry
      return { db = db, fx = { { type = "state/save", namespace = "preferences", data = preferences } } }
    end)

    misa.reg_event("preferences/toggle", function(db, event)
      if type(event.scope) ~= "string" or type(event.value) ~= "string" or type(event.items) ~= "table" then return end
      local preferences = state(db)
      local values = scope_state(preferences, event.scope)
      local entry = values[event.value] or { uses = 0 }
      entry.favorite = not (entry.favorite == true)
      values[event.value] = entry
      return { db = db, fx = {
        { type = "state/save", namespace = "preferences", data = preferences },
        { type = "dispatch", event = {
          type = "picker/update", id = event.picker, token = event.picker_token,
          panels = panels(preferences, event.scope, event.items), selected = event.selected,
        } },
        -- Favorite consumes a terminal event without closing the picker.
        { type = "terminal/read" },
      } }
    end)
  end,
}
