-- Animation registry and timer-backed service. Registrations are immutable once
-- app/start begins; selections, role ticks, and running timers are transactional.
return {
  setup = function(context)
    local entries, registrations_sealed = {}, false
    local config = type(context.config) == "table" and context.config.animations or nil
    config = type(config) == "table" and config or {}
    local configured = type(config.default) == "string" and config.default or "default"
    local configured_roles = type(config.roles) == "table" and config.roles or {}
    local interval_ms = config.interval_ms or 120
    assert(type(interval_ms) == "number" and interval_ms >= 10 and interval_ms <= 60000 and interval_ms % 1 == 0,
      "animations.interval_ms must be an integer from 10 through 60000")

    misa.reg_animation = function(id, animation)
      assert(not registrations_sealed, "animation registrations are sealed")
      assert(type(id) == "string" and id ~= "" and type(animation) == "table", "invalid animation")
      assert(type(animation.frames) == "table" and #animation.frames > 0, "animation frames must be nonempty")
      for _, frame in ipairs(animation.frames) do assert(type(frame) == "string", "animation frames must be strings") end
      assert(entries[id] == nil, "duplicate animation: " .. id)
      entries[id] = animation
    end
    local function animation_id(db, role)
      local state = assert(db.animations, "animation state is not initialized")
      return role and state.roles[role] or state.active
    end
    misa.animation = function(db, role)
      assert(type(db) == "table", "animation resolution requires initialized db")
      local id = animation_id(db, role)
      return assert(entries[id], "unknown animation: " .. tostring(id))
    end
    misa.animation_frame = function(db, role, tick)
      local frames = misa.animation(db, role).frames
      local current = tick
      if current == nil then current = (db.animations.ticks or {})[role or "default"] or 0 end
      return frames[current % #frames + 1]
    end
    misa.swap_animation = function(db, id, role)
      assert(entries[id], "unknown animation: " .. tostring(id))
      if role then db.animations.roles[role] = id else db.animations.active = id end
    end
    local timer_id = "animation/service"
    local function has_running(running) for _ in pairs(running) do return true end; return false end
    local function start_role(db, role)
      local running = db.animations.running
      if running[role] then return {} end
      local start_timer = not has_running(running); running[role] = true
      return start_timer and { { type = "timer/start", interval_ms = interval_ms, completion = "animations/tick", id = timer_id } } or {}
    end
    local function stop_role(db, role)
      local running = db.animations.running
      if not running[role] then return {} end
      running[role] = nil
      return not has_running(running) and { { type = "timer/stop", id = timer_id } } or {}
    end

    misa.reg_interceptor({ id = "animations/initialize", before = function(tx)
      if tx.event.type == "app/start" then
        registrations_sealed = true
        if not tx.db.animations then
          local roles = {}
          for role, id in pairs(configured_roles) do
            assert(type(role) == "string" and role ~= "" and type(id) == "string" and entries[id], "invalid configured animation role")
            roles[role] = id
          end
          tx.db.animations = { active = configured, roles = roles, ticks = {}, running = {} }
        end
      end
      return tx
    end })
    misa.reg_event("app/start", function(db)
      assert(entries[db.animations.active], "unknown configured animation: " .. tostring(db.animations.active))
      if config.persist == false then return { db = db } end
      return { db = db, fx = { { type = "state/load", namespace = "ui.animation", completion = "animations/loaded" } } }
    end)
    misa.reg_event("animations/loaded", function(db, event)
      if event.found == false or event.data == misa.json_null then return { db = db } end
      assert(type(event.data) == "table" and type(event.data.active) == "string", "invalid persisted animation")
      if entries[event.data.active] then db.animations.active = event.data.active end
      for role, id in pairs(type(event.data.roles) == "table" and event.data.roles or {}) do
        if type(role) == "string" and entries[id] then db.animations.roles[role] = id end
      end
      return { db = db }
    end)
    misa.reg_event("animations/swap", function(db, event)
      misa.swap_animation(db, event.animation, event.role)
      local fx = {}
      if config.persist ~= false then fx[#fx + 1] = { type = "state/save", namespace = "ui.animation", data = { active = db.animations.active, roles = db.animations.roles } } end
      fx[#fx + 1] = { type = "dispatch", event = { type = "ui/redraw" } }
      return { db = db, fx = fx }
    end)
    misa.reg_event("animations/start", function(db, event) return { db = db, fx = start_role(db, assert(event.role, "animation role is required")) } end)
    misa.reg_event("animations/stop", function(db, event) return { db = db, fx = stop_role(db, assert(event.role, "animation role is required")) } end)
    misa.reg_event("animations/tick", function(db, event)
      if event.id ~= timer_id or not has_running(db.animations.running) then return end
      -- Ignore native tick counters: every running role advances exactly once
      -- in the same transaction as the redraw request.
      for role in pairs(db.animations.running) do db.animations.ticks[role] = (db.animations.ticks[role] or 0) + 1 end
      return { db = db, fx = { { type = "dispatch", event = { type = "ui/redraw" } } } }
    end)
    misa.reg_event("agent/status", function(db, event)
      local fx = event.status == "ready" and stop_role(db, "status") or start_role(db, "status")
      return { db = db, fx = fx }
    end)
  end,
}
