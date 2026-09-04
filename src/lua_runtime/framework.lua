-- Trusted event framework. This file is embedded by Zig and is never loaded at runtime.
local traceback = debug.traceback
local events, interceptors, cofx_fns, cofx_order, fx_fns = {}, {}, {}, {}, {}
local view, sealed, dispatching, db, pending_db, base_context = nil, false, false, {}, nil, nil
local MAX_DEPTH = 128

misa = { json_null = {} }
local function open() assert(not sealed, "registrations are sealed") end
function misa.reg_event(name, fn)
  open(); assert(type(name) == "string" and name ~= ""); assert(type(fn) == "function")
  local handlers = events[name] or {}; events[name] = handlers; handlers[#handlers + 1] = fn
end
function misa.reg_interceptor(value)
  open(); assert(type(value) == "table" and type(value.id) == "string" and value.id ~= "")
  assert(value.before == nil or type(value.before) == "function")
  assert(value.after == nil or type(value.after) == "function")
  interceptors[#interceptors + 1] = value
end
function misa.reg_cofx(name, fn)
  open(); assert(type(name) == "string" and name ~= "" and name ~= "config" and name ~= "config_json" and name ~= "argv" and name ~= "terminal")
  assert(type(fn) == "function"); assert(cofx_fns[name] == nil, "duplicate cofx")
  cofx_fns[name] = fn; cofx_order[#cofx_order + 1] = name
end
function misa.reg_fx(name, fn)
  open(); assert(type(name) == "string" and name ~= ""); assert(type(fn) == "function")
  assert(fx_fns[name] == nil, "duplicate fx"); fx_fns[name] = fn
end
function misa.reg_view(fn)
  open(); assert(type(fn) == "function"); assert(view == nil, "view already registered"); view = fn
end

local function esc(value)
  return '"' .. value:gsub('[%z\1-\31\\"]', function(char)
    local byte = char:byte()
    if char == '"' then return '\\"' elseif char == '\\' then return '\\\\'
    elseif char == '\n' then return '\\n' elseif char == '\r' then return '\\r'
    elseif char == '\t' then return '\\t' else return string.format('\\u%04x', byte) end
  end) .. '"'
end
local function finite(value) return value == value and value ~= math.huge and value ~= -math.huge end
local function clone(value, active, depth)
  depth = depth or 0; assert(depth <= MAX_DEPTH, "maximum nesting depth exceeded")
  local kind = type(value)
  if value == misa.json_null or kind == "nil" or kind == "boolean" or kind == "string" then return value end
  if kind == "number" then assert(finite(value), "non-finite number"); return value end
  assert(kind == "table", "state must contain only JSON values")
  active = active or {}; assert(not active[value], "cyclic state"); active[value] = true
  local result, count, maximum, key_shape = {}, 0, 0, nil
  for key in pairs(value) do
    local key_kind = type(key)
    assert(key_kind == "string" or (key_kind == "number" and finite(key) and key >= 1 and key % 1 == 0), "invalid table key")
    local shape = key_kind == "string" and "object" or "array"
    assert(key_shape == nil or key_shape == shape, "table mixes object and array keys")
    key_shape, count = shape, count + 1
    if shape == "array" and key > maximum then maximum = key end
  end
  assert(key_shape ~= "array" or maximum == count, "array state must be contiguous")
  for key, item in pairs(value) do result[key] = clone(item, active, depth + 1) end
  active[value] = nil
  return result
end
local function encode(value, active, depth)
  depth = depth or 0; assert(depth <= MAX_DEPTH, "maximum nesting depth exceeded")
  local kind = type(value)
  if value == misa.json_null or kind == "nil" then return "null"
  elseif kind == "boolean" then return tostring(value)
  elseif kind == "number" then assert(finite(value), "non-finite number"); return tostring(value)
  elseif kind == "string" then return esc(value) end
  assert(kind == "table", "event data must contain JSON values")
  active = active or {}; assert(not active[value], "cyclic event data"); active[value] = true
  local count, maximum, array = 0, 0, true
  for key in pairs(value) do
    count = count + 1
    if type(key) ~= "number" or not finite(key) or key < 1 or key % 1 ~= 0 then array = false
    elseif key > maximum then maximum = key end
  end
  local out = {}
  if array and maximum == count then
    for i = 1, maximum do out[#out + 1] = encode(value[i], active, depth + 1) end
    active[value] = nil; return "[" .. table.concat(out, ",") .. "]"
  end
  for key, item in pairs(value) do
    assert(type(key) == "string", "object keys must be strings")
    out[#out + 1] = esc(key) .. ":" .. encode(item, active, depth + 1)
  end
  active[value] = nil; return "{" .. table.concat(out, ",") .. "}"
end
local function append(destination, values)
  assert(type(values) == "table", "fx must be a table")
  for i = 1, #values do assert(type(values[i]) == "table", "fx entries must be tables"); destination[#destination + 1] = values[i] end
end
-- Zig captures this bounded clone bridge in the registry and removes the field
-- before loading extensions. Keep cloning policy in one trusted implementation.
function misa._clone(value) return clone(value) end
function misa._seal(context) sealed = true; base_context = clone(context) end
function misa._dispatch(event, terminal)
  assert(sealed, "registrations are not sealed"); assert(not dispatching, "recursive dispatch is forbidden")
  assert(pending_db == nil, "previous transaction was not committed")
  assert(type(event) == "table" and type(event.type) == "string" and event.type ~= "", "event.type must be a nonempty string")
  assert(type(terminal) == "table" and type(terminal.interactive) == "boolean", "invalid terminal coeffect")
  assert(type(terminal.columns) == "number" and terminal.columns >= 1 and terminal.columns % 1 == 0, "invalid terminal columns")
  assert(type(terminal.lines) == "number" and terminal.lines >= 1 and terminal.lines % 1 == 0, "invalid terminal lines")
  dispatching = true
  local ok, result = xpcall(function()
    -- Every transaction gets an isolated base snapshot. Extension mutation is
    -- therefore transaction-local, including when native validation rejects it.
    local cofx = {
      config = clone(base_context.config),
      config_json = clone(base_context.config_json),
      argv = clone(base_context.argv),
      terminal = clone(terminal),
    }
    local working = clone(db)
    -- Derived coeffects are intentionally threaded in registration order, so a
    -- later derivation may read values installed by earlier derivations.
    for _, name in ipairs(cofx_order) do cofx[name] = cofx_fns[name](cofx, event, working) end
    local tx = { db = working, event = event, cofx = cofx, fx = {} }
    local function valid_tx(value)
      assert(type(value) == "table" and type(value.db) == "table" and type(value.cofx) == "table" and type(value.fx) == "table", "invalid interceptor transaction")
    end
    for i = 1, #interceptors do if interceptors[i].before then tx = interceptors[i].before(tx) or tx; valid_tx(tx) end end
    for _, fn in ipairs(events[tx.event.type] or {}) do
      local value = fn(tx.db, tx.event, tx.cofx)
      assert(value == nil or type(value) == "table", "event handler result must be a table")
      if value then
        if value.db ~= nil then assert(type(value.db) == "table", "handler db must be a table"); tx.db = value.db end
        if value.fx ~= nil then append(tx.fx, value.fx) end
      end
    end
    for i = #interceptors, 1, -1 do if interceptors[i].after then tx = interceptors[i].after(tx) or tx; valid_tx(tx) end end
    local native = {}
    for _, effect in ipairs(tx.fx) do
      local kind = effect.type; assert(type(kind) == "string" and kind ~= "", "effect.type must be a nonempty string")
      local translator = fx_fns[kind]
      if translator then
        local translated = translator(effect, tx.cofx, tx.db); assert(type(translated) == "table", "fx translator must return a table")
        if translated.type then native[#native + 1] = translated else append(native, translated) end
      else native[#native + 1] = effect end
    end
    local projection = misa.json_null
    if view then projection = view(tx.db, tx.cofx); assert(type(projection) == "table", "view must return a table") end
    local encoded = encode({ fx = native, view = projection })
    pending_db = clone(tx.db)
    return encoded
  end, traceback)
  dispatching = false
  if not ok then error(result, 0) end
  return result
end
function misa._commit()
  assert(pending_db ~= nil, "no transaction to commit")
  db, pending_db = pending_db, nil
end

-- Remove every standard direct output, process, native-code, dynamic loading,
-- and termination route before untrusted extension chunks are evaluated.
os = nil; io = nil; print = nil; package = nil; require = nil
load = nil; loadstring = nil; loadfile = nil; dofile = nil
ffi = nil; jit = nil; debug = nil; module = nil
