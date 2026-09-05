-- Reasoning-effort affordances over the generic request-options policy.
local option_name = "reasoning_effort"

local function choices(db)
  return misa.request_option_choices and misa.request_option_choices(db, option_name) or {}
end

local function unavailable(db)
  local model = db.models and db.models.selected or "selected model"
  return { db = db, fx = {
    { type = "dispatch", event = {
      type = "transcript/harness", level = "info",
      text = "reasoning effort is not supported by " .. model,
      problem = { kind = "request_option", code = "unsupported", option = option_name, model = model },
    } },
    { type = "terminal/read" },
  } }
end

return { setup = function()
  assert(misa.request_option_choices, "effort requires request_options first")
  misa.reg_keybinding({ context = "global", action = "cycle_effort", default = { "alt+e" } })
  if misa.reg_indicator then misa.reg_indicator({
    id="effort", label="effort", icon="◈",
    hotkey={context="global",action="cycle_effort"},
    value=function(db) return misa.request_option_value(db,option_name) end,
  }) end
  misa.reg_command({
    name = "/effort", description = "Choose model reasoning effort", event = "effort/select",
    preference_scope = "request-options/effort", choice_purpose = "command",
    selected = function(db) return misa.request_option_value(db, option_name) end,
    complete = function(_, db)
      local result = {}
      for _, value in ipairs(choices(db)) do result[#result + 1] = { value = tostring(value), label = tostring(value) } end
      return result
    end,
  })

  -- Intercept the shared command-choice transaction when effort is unsupported.
  misa.reg_interceptor({ id = "effort/input", before = function(tx)
    if tx.event.type == "choices/command-open" and tx.event.command == "/effort" and #choices(tx.db) == 0 then
      tx.event = { type = "effort/unsupported" }
    elseif tx.event.type == "terminal/input" and misa.keybinding_action and misa.keybinding_action("global", tx.event) == "cycle_effort" then
      tx.event = { type = "effort/cycle" }
    end
    return tx
  end })

  misa.reg_event("effort/unsupported", function(db) return unavailable(db) end)
  misa.reg_event("effort/select", function(db, event)
    local available = choices(db)
    if #available == 0 then return unavailable(db) end
    local requested = type(event.arguments) == "string" and event.arguments:match("^%s*(%S+)%s*$") or nil
    for _, value in ipairs(available) do
      if tostring(value) == requested then
        return { db = db, fx = {
          { type = "dispatch", event = { type = "request-options/select", name = option_name, value = value } },
          { type = "terminal/read" },
        } }
      end
    end
    error("unsupported reasoning effort for the selected model")
  end)

  misa.reg_event("effort/cycle", function(db)
    local available = choices(db)
    if #available == 0 then return { db = db, fx = { { type = "terminal/read" } } } end
    local current = misa.request_option_value(db, option_name)
    local index = 0
    for i, value in ipairs(available) do if value == current then index = i; break end end
    local value = available[index % #available + 1]
    return { db = db, fx = {
      { type = "dispatch", event = { type = "request-options/select", name = option_name, value = value } },
      { type = "terminal/read" },
    } }
  end)
end }
