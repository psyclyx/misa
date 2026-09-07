-- Run from the repository root with luajit benchmarks/startup.lua.
-- CPU-time phase attribution, not end-to-end interactive startup latency.
-- Native effects are collected but never executed.
local clock, write, open, loader = os.clock, io.write, io.open, loadstring
local precompiled = os.getenv("MISA_STARTUP_PRECOMPILED")
local started = clock()
local environment = {}
for key, value in pairs(_G) do environment[key] = value end
local compiler = setfenv(assert(loadfile("src/lua_runtime/vendor/fennel.lua")), environment)()
local bootstrap = clock() - started
package.loaded.fennel = compiler
compiler.install()
local rows, totals = {}, {compile = 0, load = 0, setup = 0}
local function source(path)
  local file = assert(open(path, "rb"))
  local text = assert(file:read("*a"))
  assert(file:close())
  return text
end
local function load_fennel(path)
  local text = source(precompiled and (precompiled .. "/" .. path:gsub("%.fnl$", ".lua")) or path)
  local start = clock()
  local lua = precompiled and text or compiler.compileString(text, {filename = path, allowedGlobals = false})
  local compiled = precompiled and 0 or clock() - start
  start = clock()
  local value = assert(loader(lua, "@" .. path))()
  local loaded = clock() - start
  totals.compile = totals.compile + compiled
  totals.load = totals.load + loaded
  rows[#rows + 1] = {path = path, compile = compiled, load = loaded}
  return value
end
package.loaded["misa.runtime.state"] = load_fennel("src/lua_runtime/state.fnl")
package.loaded["misa.runtime.subscriptions"] = load_fennel("src/lua_runtime/subscriptions.fnl")
load_fennel("src/lua_runtime/framework.fnl")
local json = load_fennel("extensions/json.fnl")
misa._setup(json, {config = {}})
local config = misa.json.decode(source("config/default.json"))
assert(config.extensions[1] == "json")
local context = {config = config.config, argv = {}, host = {executable = "misa"}}
for index = 2, #config.extensions do
  local path = "extensions/" .. config.extensions[index]:gsub("%.", "/") .. ".fnl"
  local extension = load_fennel(path)
  local start = clock()
  misa._setup(extension, context)
  local elapsed = clock() - start
  rows[#rows].setup = elapsed
  totals.setup = totals.setup + elapsed
end
misa._seal(context)
local start = clock()
local effects, view = misa._dispatch({type = "app/start"},
  {columns = 80, lines = 24, interactive = false, images = false},
  {wall_ms = 0, monotonic_ms = 0})
misa._commit()
local dispatch = clock() - start
-- Deterministic semantic output lets the caller check repeated runs before
-- trusting their timing. Do not serialize pointer addresses or callbacks.
local oracle = misa.json.encode({effects = effects, view = view,
                                models = #misa.models(), commands = #misa.commands()})
write(misa.json.encode({bootstrap = bootstrap, compile = totals.compile,
                       load = totals.load, setup = totals.setup, dispatch = dispatch,
                       total = clock() - started, rows = rows, oracle = oracle}), "\n")
