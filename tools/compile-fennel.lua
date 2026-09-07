-- Build-time source translation. Generated artifacts are ordinary Lua, not
-- architecture-specific LuaJIT bytecode. Arguments are input/output pairs.
local fennel = dofile("src/lua_runtime/vendor/fennel.lua")
assert(#arg > 0 and #arg % 2 == 0, "expected input/output path pairs")
for index = 1, #arg, 2 do
  local input = assert(io.open(arg[index], "rb"))
  local source = assert(input:read("*a"))
  assert(input:close())
  local generated = fennel.compileString(source, {
    filename = arg[index], allowedGlobals = false, correlate = true,
  })
  local output = assert(io.open(arg[index + 1], "wb"))
  assert(output:write(generated, "\n"))
  assert(output:close())
end
