-- Semantic visual component registry. Implementations are immutable registration
-- data; every role selection lives in transactional application state.
return {
  setup = function(context)
    local implementations, registrations_sealed = {}, false
    local config = type(context.config) == "table" and context.config.components or nil
    config = type(config) == "table" and config or {}
    local configured = type(config.roles) == "table" and config.roles or config

    misa.reg_component = function(id, component)
      assert(not registrations_sealed, "component registrations are sealed")
      assert(type(id) == "string" and id ~= "", "component ID must be nonempty")
      assert(type(component) == "table" and type(component.render) == "function", "component must provide render")
      assert(implementations[id] == nil, "duplicate component: " .. id)
      implementations[id] = component
    end
    misa.component = function(db, role)
      assert(type(db) == "table", "component resolution requires db")
      assert(type(role) == "string" and role ~= "", "component role must be nonempty")
      local state = assert(db.components, "component state is not initialized")
      local id = state.roles[role] or ("default." .. role)
      assert(type(id) == "string" and id ~= "", "no component configured for role: " .. role)
      return assert(implementations[id], "unknown component for " .. role .. ": " .. id)
    end
    local native_style = { plain="plain", dim="dim", bold="bold", accent="accent", user="user", assistant="assistant", error="error", thinking="dim", tool="dim" }
    misa.render_component = function(db, role, model, render_context)
      local component = misa.component(db, role)
      local rendered = component.render(misa.snapshot(model), misa.snapshot(render_context or {}))
      assert(type(rendered) == "table", "component render must return a table")
      -- Components emit semantic tokens and know nothing about the active theme.
      -- Resolution is centralized at the registry boundary before native validation.
      for _, line in ipairs(rendered.lines or {}) do for _, span in ipairs(line.spans or {}) do
        local fallback = native_style[span.style] or "plain"
        span.style = misa.theme_token and misa.theme_token(db, span.style, fallback) or fallback
      end end
      return rendered
    end
    misa.swap_component = function(db, role, id)
      assert(type(role) == "string" and role ~= "" and implementations[id], "unknown component: " .. tostring(id))
      db.components.roles[role] = id
    end

    misa.reg_interceptor({ id = "components/initialize", before = function(tx)
      if tx.event.type == "app/start" then
        registrations_sealed = true
        if not tx.db.components then
        local roles = {}
        for role, id in pairs(configured) do
          if role ~= "persist" then
            assert(type(role) == "string" and type(id) == "string", "invalid configured component role")
            roles[role] = id
          end
        end
        tx.db.components = { roles = roles }
        end
      end
      return tx
    end })
    misa.reg_event("app/start", function(db)
      if config.persist == false then return { db = db } end
      return { db = db, fx = { { type = "state/load", namespace = "ui.components", completion = "components/loaded" } } }
    end)
    misa.reg_event("components/loaded", function(db, event)
      if event.found == false or event.data == misa.json_null then return { db = db } end
      local saved = event.data
      assert(type(saved) == "table" and type(saved.roles) == "table", "invalid persisted component selections")
      for role, id in pairs(saved.roles) do
        assert(type(role) == "string" and role ~= "" and type(id) == "string", "invalid persisted component selection")
        if implementations[id] then db.components.roles[role] = id end
      end
      return { db = db }
    end)
    misa.reg_event("components/swap", function(db, event)
      assert(type(event.role) == "string" and type(event.implementation) == "string", "invalid component swap")
      misa.swap_component(db, event.role, event.implementation)
      local fx = {}
      if config.persist ~= false then fx[#fx + 1] = { type = "state/save", namespace = "ui.components", data = db.components } end
      fx[#fx + 1] = { type = "dispatch", event = { type = "ui/redraw" } }
      return { db = db, fx = fx }
    end)
  end,
}
