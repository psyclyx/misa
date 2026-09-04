-- Generic per-model request-option selection and readiness. Model providers
-- declare API capabilities; this extension alone owns their mutable values.
local function selected_model(db)
  local models = db.models
  if not models then return nil end
  for _, model in ipairs(models.entries or {}) do
    if model.id == models.selected then return model end
  end
end

local function declarations(model)
  local api = model and model.api
  return type(api) == "table" and type(api.request_options) == "table" and api.request_options or {}
end

local function choices(option)
  if type(option) ~= "table" then return {} end
  local values = option.choices or option.values
  return type(values) == "table" and values or {}
end

local function equivalent(left, right)
  return type(left) == type(right) and left == right
end

local function contains(values, value)
  if value == nil then return false end
  for _, candidate in ipairs(values) do if equivalent(candidate, value) then return true end end
  return false
end

local function valid(option, value)
  if value == nil or type(option) ~= "table" then return false end
  local values = choices(option)
  return #values == 0 or contains(values, value)
end

local function reconcile(db)
  db.request_options = db.request_options or { values = {} }
  local state, model = db.request_options, selected_model(db)
  local previous = state.values or {}
  local next_values, configured = {}, state.configured or {}
  for name, option in pairs(declarations(model)) do
    if type(name) == "string" and type(option) == "table" then
      local value = previous[name]
      if not valid(option, value) then
        value = state.model_id == nil and configured[name] or nil
        if not valid(option, value) then value = option.default end
      end
      if valid(option, value) then next_values[name] = value end
    end
  end
  state.values, state.model_id = next_values, model and model.id or nil
  return state
end

local function prepare(db, model)
  local state = reconcile(db)
  model = model or selected_model(db)
  local result, problems = {}, {}
  local serializer = model and type(model.api) == "table" and model.api.request_options_serializer or nil
  for name, option in pairs(declarations(model)) do
    if type(name) ~= "string" or name == "" or type(option) ~= "table" then
      problems[#problems + 1] = { option = tostring(name), reason = "invalid_declaration" }
    else
      local value = state.values[name]
      if (value ~= nil or option.required == true) and not misa.can_serialize_request_option(serializer, name) then
        problems[#problems + 1] = { option = name, reason = "not_serializable" }
      elseif value ~= nil and valid(option, value) then result[name] = value
      elseif option.required == true then problems[#problems + 1] = { option = name, reason = value == nil and "missing" or "unsupported_value" }
      end
    end
  end
  if #problems == 0 then return result end
  table.sort(problems, function(left, right) return left.option < right.option end)
  local names, unserializable = {}, false
  for _, item in ipairs(problems) do names[#names + 1] = item.option; if item.reason == "not_serializable" then unserializable = true end end
  local prefix = unserializable and "request options cannot be serialized: " or "required request options are missing: "
  return nil, {
    kind = "request_readiness", code = unserializable and "unserializable_request_options" or "missing_required_request_options",
    model = model and model.id or nil, missing = problems,
    message = prefix .. table.concat(names, ", "),
  }
end

return { setup = function(context)
  local config = type(context.config) == "table" and context.config.request_options or nil
  config = type(config) == "table" and config or {}
  local configured = type(config.values) == "table" and config.values or config

  misa.request_option_choices = function(db, name)
    local option = declarations(selected_model(db))[name]
    local result = {}; for _, value in ipairs(choices(option)) do result[#result + 1] = value end
    return result
  end
  misa.request_option_value = function(db, name)
    return reconcile(db).values[name]
  end
  misa.request_options_projection = function(db)
    local state = db.request_options or {}; local values = {}
    for name, value in pairs(state.values or {}) do values[name] = value end
    return { model_id=state.model_id, values=values }
  end
  misa.reconcile_request_options = reconcile
  misa.prepare_request_options = prepare

  misa.reg_event("app/start", function(db)
    db.request_options = { values = {}, configured = configured }
    reconcile(db)
    return { db = db }
  end)

  for _, event_type in ipairs({ "model/open", "model/select", "models/provider-availability", "models/update", "models/replace-provider" }) do
    misa.reg_event(event_type, function(db)
      if db.request_options then reconcile(db) end
      return { db = db }
    end)
  end

  misa.reg_event("request-options/select", function(db, event)
    assert(type(event.name) == "string" and event.name ~= "", "request option name must be nonempty")
    local option = declarations(selected_model(db))[event.name]
    assert(type(option) == "table", "request option is not supported by the selected model")
    assert(valid(option, event.value), "request option value is not supported by the selected model")
    local state = reconcile(db)
    state.values[event.name] = event.value
    return { db = db }
  end)
end }
