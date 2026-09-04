-- Theme data registry. Theme selection is transactional application state.
return {
  setup = function(context)
    local entries, registrations_sealed = {}, false
    local config = type(context.config) == "table" and context.config.themes or nil
    config = type(config) == "table" and config or {}
    local configured = type(config.default) == "string" and config.default or "default"

    misa.reg_theme = function(id, theme)
      assert(not registrations_sealed, "theme registrations are sealed")
      assert(type(id) == "string" and id ~= "" and type(theme) == "table", "invalid theme")
      assert(entries[id] == nil, "duplicate theme: " .. id)
      entries[id] = theme
    end
    misa.theme = function(db)
      assert(type(db) == "table" and type(db.themes) == "table", "theme resolution requires initialized db")
      return assert(entries[db.themes.active], "unknown theme: " .. tostring(db.themes.active))
    end
    misa.theme_token = function(db, name, fallback)
      local value = misa.theme(db)[name]
      return type(value) == "string" and value or fallback
    end
    misa.swap_theme = function(db, id)
      assert(entries[id], "unknown theme: " .. tostring(id))
      db.themes.active = id
    end
    misa.reg_interceptor({ id = "themes/initialize", before = function(tx)
      if tx.event.type == "app/start" then
        registrations_sealed = true
        if not tx.db.themes then tx.db.themes = { active = configured } end
      end
      return tx
    end })
    misa.reg_event("app/start", function(db)
      if config.persist == false then return { db = db } end
      return { db = db, fx = { { type = "state/load", namespace = "ui.theme", completion = "themes/loaded" } } }
    end)
    misa.reg_event("themes/loaded", function(db, event)
      if event.found == false or event.data == misa.json_null then return { db = db } end
      assert(type(event.data) == "table" and type(event.data.active) == "string", "invalid persisted theme")
      if entries[event.data.active] then db.themes.active = event.data.active end
      return { db = db }
    end)
    misa.reg_event("themes/swap", function(db, event)
      misa.swap_theme(db, event.theme)
      local fx = {}
      if config.persist ~= false then fx[#fx + 1] = { type = "state/save", namespace = "ui.theme", data = db.themes } end
      fx[#fx + 1] = { type = "dispatch", event = { type = "ui/redraw" } }
      return { db = db, fx = fx }
    end)
  end,
}
