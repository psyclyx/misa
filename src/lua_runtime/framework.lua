-- Trusted event framework, embedded by Zig.
local traceback = debug.traceback
local events, interceptors, interceptor_ids = {}, {}, {}
local cofx_fns, cofx_order, fx_fns = {}, {}, {}
local models, model_by_id, tools, tool_by_name = {}, {}, {}, {}
local commands, command_by_name = {}, {}
local completions, completion_values = {}, {}
local auth_providers, auth_provider_ids, auth_model_providers = {}, {}, {}
local view, sealed, dispatching, db, pending_db, base_context = nil, false, false, {}, nil, nil
local MAX_DEPTH = 128

misa = { json_null = {} }
local function open() assert(not sealed, "registrations are sealed") end

function misa.reg_event(name, fn)
  open()
  assert(type(name) == "string" and name ~= "")
  assert(type(fn) == "function")
  local handlers = events[name] or {}
  events[name] = handlers
  handlers[#handlers + 1] = fn
end

function misa.reg_interceptor(value)
  open()
  assert(type(value) == "table" and type(value.id) == "string" and value.id ~= "")
  assert(not interceptor_ids[value.id], "duplicate interceptor")
  assert(value.before == nil or type(value.before) == "function")
  assert(value.after == nil or type(value.after) == "function")
  interceptor_ids[value.id] = true
  interceptors[#interceptors + 1] = value
end

function misa.reg_cofx(name, fn)
  open()
  assert(type(name) == "string" and name ~= "")
  assert(name ~= "config" and name ~= "argv" and name ~= "terminal")
  assert(type(fn) == "function" and cofx_fns[name] == nil, "duplicate cofx")
  cofx_fns[name] = fn
  cofx_order[#cofx_order + 1] = name
end

function misa.reg_fx(name, fn)
  open()
  assert(type(name) == "string" and name ~= "")
  assert(type(fn) == "function" and fx_fns[name] == nil, "duplicate fx")
  fx_fns[name] = fn
end

function misa.reg_view(fn)
  open()
  assert(type(fn) == "function" and view == nil, "view already registered")
  view = fn
end

function misa.reg_model(model)
  open()
  assert(type(model) == "table" and type(model.id) == "string" and model.id ~= "", "model.id must be a nonempty string")
  assert(type(model.provider) == "string" and model.provider ~= "", "model.provider must be a nonempty string")
  assert(type(model.model) == "string" and model.model ~= "", "model.model must be a nonempty string")
  assert(model.label == nil or type(model.label) == "string", "model.label must be a string")
  assert(model.context_window == nil or (type(model.context_window) == "number" and model.context_window > 0 and model.context_window % 1 == 0), "model.context_window must be a positive integer")
  assert(model_by_id[model.id] == nil, "duplicate model")
  model_by_id[model.id] = model
  models[#models + 1] = model
end

function misa.models() return models end
function misa.model(id) return model_by_id[id] end

function misa.reg_command(command)
  open()
  assert(type(command) == "table" and type(command.name) == "string" and command.name:match("^/[%w_-]+$"), "command.name must look like /name")
  assert(type(command.description) == "string", "command.description must be a string")
  assert(type(command.event) == "string" and command.event ~= "", "command.event must be nonempty")
  assert(command.completion == nil or type(command.completion) == "string", "command.completion must name a completion group")
  assert(command.complete == nil or type(command.complete) == "function", "command.complete must be a function")
  assert(not (command.completion and command.complete), "command may have one completion source")
  assert(command_by_name[command.name] == nil, "duplicate command")
  command_by_name[command.name] = command
  commands[#commands + 1] = command
end

function misa.commands() return commands end
function misa.command(name) return command_by_name[name] end

function misa.reg_completion(group, candidate)
  open()
  assert(type(group) == "string" and group ~= "", "completion group must be nonempty")
  assert(type(candidate) == "table" and type(candidate.value) == "string" and candidate.value ~= "", "completion value must be nonempty")
  assert(candidate.label == nil or type(candidate.label) == "string", "completion label must be a string")
  assert(candidate.description == nil or type(candidate.description) == "string", "completion description must be a string")
  completions[group] = completions[group] or {}
  completion_values[group] = completion_values[group] or {}
  assert(not completion_values[group][candidate.value], "duplicate completion value")
  completion_values[group][candidate.value] = true
  completions[group][#completions[group] + 1] = candidate
end

function misa.reg_auth_provider(provider)
  open()
  assert(type(provider) == "table" and type(provider.id) == "string" and provider.id ~= "", "auth provider ID must be nonempty")
  assert(type(provider.model_provider) == "string" and provider.model_provider ~= "", "auth model provider must be nonempty")
  assert(provider.discover_models == nil or type(provider.discover_models) == "boolean", "auth provider discover_models must be boolean")
  assert(not auth_provider_ids[provider.id], "duplicate auth provider")
  assert(not auth_model_providers[provider.model_provider], "duplicate auth model provider")
  auth_provider_ids[provider.id] = true
  auth_model_providers[provider.model_provider] = true
  auth_providers[#auth_providers + 1] = provider
  misa.reg_completion("auth-provider", { value = provider.id, label = provider.label, description = provider.description })
end
function misa.auth_providers() return auth_providers end

function misa.command_completions(command, prefix, state)
  assert(type(command) == "table" and type(prefix) == "string", "invalid completion request")
  local source = command.complete and command.complete(prefix, state) or completions[command.completion] or {}
  assert(type(source) == "table", "command completer must return an array")
  local result = {}
  for _, candidate in ipairs(source) do
    assert(type(candidate) == "table" and type(candidate.value) == "string", "invalid completion candidate")
    if candidate.value:sub(1, #prefix) == prefix then result[#result + 1] = candidate end
  end
  table.sort(result, function(left, right) return left.value < right.value end)
  return result
end

function misa.reg_tool(tool)
  open()
  assert(type(tool) == "table" and type(tool.name) == "string" and tool.name ~= "", "tool.name must be a nonempty string")
  assert(type(tool.description) == "string", "tool.description must be a string")
  assert(type(tool.input_schema) == "table" and tool.input_schema.type == "object", "tool.input_schema must be an object schema")
  assert(type(tool.effect) == "string" and tool.effect ~= "", "tool.effect must be a nonempty string")
  assert(tool_by_name[tool.name] == nil, "duplicate tool")
  tool_by_name[tool.name] = tool
  tools[#tools + 1] = tool
end

function misa.tools() return tools end
function misa.tool(name) return tool_by_name[name] end

-- The MCP bridge asks Lua for schemas and translates calls through the same
-- registered tool/effect policy used by the interactive agent.
function misa._mcp_tools() return tools end
function misa._mcp_tool_effect(name, arguments, id)
  assert(sealed, "registrations are not sealed")
  local tool = assert(tool_by_name[name], "unknown tool: " .. tostring(name))
  assert(type(arguments) == "table", "tool arguments must be an object")
  local translator = assert(fx_fns[tool.effect], "tool effect has no translator: " .. tool.effect)
  local translated = translator({
    type = tool.effect, arguments = arguments, tool_call_id = id,
    request_id = "mcp", name = name,
  }, {
    config = base_context.config, argv = base_context.argv,
    terminal = { interactive = false, columns = 80, lines = 24 },
  }, db)
  assert(type(translated) == "table" and type(translated.type) == "string", "MCP tool translator must return one native effect")
  return translated
end

local function finite(value)
  return value == value and value ~= math.huge and value ~= -math.huge
end

-- One working copy gives a transaction exclusive state without repeatedly
-- cloning immutable configuration and coeffects.
local function clone(value, active, depth)
  depth = depth or 0
  assert(depth <= MAX_DEPTH, "maximum state nesting depth exceeded")
  local kind = type(value)
  if value == misa.json_null or kind == "nil" or kind == "boolean" or kind == "string" then return value end
  if kind == "number" then assert(finite(value), "non-finite number"); return value end
  assert(kind == "table", "state must contain only data")
  active = active or {}
  assert(not active[value], "cyclic state")
  active[value] = true
  local result = {}
  for key, item in pairs(value) do
    local key_kind = type(key)
    assert(key_kind == "string" or (key_kind == "number" and finite(key) and key >= 1 and key % 1 == 0), "invalid state key")
    result[key] = clone(item, active, depth + 1)
  end
  active[value] = nil
  return result
end

local function append(destination, values)
  assert(type(values) == "table", "fx must be an array")
  for i = 1, #values do
    assert(type(values[i]) == "table", "fx entries must be tables")
    destination[#destination + 1] = values[i]
  end
end

function misa._seal(context)
  sealed = true
  base_context = context
end

function misa._dispatch(event, terminal)
  assert(sealed and not dispatching, "invalid dispatch state")
  assert(pending_db == nil, "previous transaction was not committed")
  assert(type(event) == "table" and type(event.type) == "string" and event.type ~= "", "event.type must be a nonempty string")
  dispatching = true
  local ok, native, projection = xpcall(function()
    local cofx = { config = base_context.config, argv = base_context.argv, terminal = terminal }
    local working = clone(db)
    for _, name in ipairs(cofx_order) do cofx[name] = cofx_fns[name](cofx, event, working) end
    local tx = { db = working, event = event, cofx = cofx, fx = {} }
    local function validate(value)
      assert(type(value) == "table" and type(value.db) == "table" and type(value.cofx) == "table" and type(value.fx) == "table", "invalid interceptor transaction")
    end
    for i = 1, #interceptors do
      local before = interceptors[i].before
      if before then tx = before(tx) or tx; validate(tx) end
    end
    for _, handler in ipairs(events[tx.event.type] or {}) do
      local result = handler(tx.db, tx.event, tx.cofx)
      assert(result == nil or type(result) == "table", "event handler result must be a table")
      if result then
        if result.db ~= nil then assert(type(result.db) == "table", "handler db must be a table"); tx.db = result.db end
        if result.fx ~= nil then append(tx.fx, result.fx) end
      end
    end
    for i = #interceptors, 1, -1 do
      local after = interceptors[i].after
      if after then tx = after(tx) or tx; validate(tx) end
    end
    local effects = {}
    for _, effect in ipairs(tx.fx) do
      local kind = effect.type
      assert(type(kind) == "string" and kind ~= "", "effect.type must be a nonempty string")
      local translator = fx_fns[kind]
      if translator then
        local translated = translator(effect, tx.cofx, tx.db)
        assert(type(translated) == "table", "fx translator must return a table")
        if translated.type then effects[#effects + 1] = translated else append(effects, translated) end
      else
        effects[#effects + 1] = effect
      end
    end
    local frame = misa.json_null
    if view then frame = view(tx.db, tx.cofx); assert(type(frame) == "table", "view must return a table") end
    pending_db = tx.db
    return effects, frame
  end, traceback)
  dispatching = false
  if not ok then error(native, 0) end
  return native, projection
end

function misa._commit()
  assert(pending_db ~= nil, "no transaction to commit")
  db, pending_db = pending_db, nil
end

-- Extensions are trusted policy. Keep ordinary Lua loading/composition, while
-- reserving terminal output, process termination, and native-library loading
-- to Zig-owned effects.
if package then
  package.loadlib = nil
  package.loaded.io, package.loaded.os, package.loaded.debug = nil, nil, nil
  package.loaded.ffi, package.loaded.jit = nil, nil
  package.preload.ffi, package.preload.jit = nil, nil
  package.loaders[3], package.loaders[4] = nil, nil
end
os, io, print, ffi, jit, debug = nil, nil, nil, nil, nil, nil
