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
local rows, by_path = {}, {}
local totals = {compile = 0, load = 0, construct = 0}
local function source(path)
  local file = assert(open(path, "rb"))
  local text = assert(file:read("*a"))
  assert(file:close())
  return text
end
local function measured()
  return totals.compile + totals.load + totals.construct
end
local function load_fennel(path)
  local text = source(precompiled and (precompiled .. "/" .. path:gsub("%.fnl$", ".lua")) or path)
  local start = clock()
  local lua = precompiled and text or compiler.compileString(text, {filename = path, allowedGlobals = false})
  local compiled = precompiled and 0 or clock() - start
  totals.compile = totals.compile + compiled
  -- require may recursively compile/load modules or construct application data.
  -- Attribute that work to its own row, not every ancestor's load interval.
  local nested = measured()
  start = clock()
  local value = assert(loader(lua, "@" .. path))()
  local loaded = math.max(0, clock() - start - (measured() - nested))
  totals.load = totals.load + loaded
  local row = {path = path, compile = compiled, load = loaded}
  rows[#rows + 1], by_path[path] = row, row
  return value
end
-- Route dependency imports through the same measured source/precompiled path.
table.insert(package.loaders, 1, function(name)
  local path = "extensions/" .. name:gsub("%.", "/") .. ".fnl"
  local file = open(path, "rb")
  if not file then return "\n\tno benchmark module '" .. path .. "'" end
  file:close()
  return function() return load_fennel(path) end
end)
package.loaded["misa.runtime.state"] = load_fennel("src/lua_runtime/state.fnl")
package.loaded["misa.runtime.subscriptions"] = load_fennel("src/lua_runtime/subscriptions.fnl")
load_fennel("src/lua_runtime/framework.fnl")
local standard = require("misa.standard")
for id, module in pairs(standard.default.modules) do
  local source_name = module.source
  local build = source_name and require(source_name) or module.build
  if type(build) == "function" then
    module.source = nil
    local path = "extensions/" .. (source_name or id):gsub("%.", "/") .. ".fnl"
    module.build = function(context)
      local nested = measured()
      local start = clock()
      local value = build(context)
      local elapsed = math.max(0, clock() - start - (measured() - nested))
      totals.construct = totals.construct + elapsed
      by_path[path].construct = (by_path[path].construct or 0) + elapsed
      return value
    end
  end
end
local application = load_fennel("config/default.fnl")
local context = {config = application.config, argv = {}, host = {executable = "misa"}}
local start = clock()
misa._install(application.definitions, context)
local installed = clock() - start
start = clock()
local effects = misa._dispatch({type = "app/start"},
  {columns = 80, lines = 24, interactive = false, images = false},
  {wall_ms = 0, monotonic_ms = 0})
misa._commit()
local view = misa._project(
  {columns = 80, lines = 24, interactive = false, images = false},
  {wall_ms = 0, monotonic_ms = 0})
misa._commit_projection()
local dispatch = clock() - start
-- Exclude pointer addresses/callbacks from the deterministic semantic oracle.
local oracle = misa.json.encode({effects = effects, view = view,
                                models = #misa.models.all(), commands = #misa.commands.all()})
write(misa.json.encode({bootstrap = bootstrap, compile = totals.compile,
                       load = totals.load, construct = totals.construct,
                       install = installed, dispatch = dispatch,
                       total = clock() - started, rows = rows, oracle = oracle}), "\n")
