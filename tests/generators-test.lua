local G = require("tests.generators")
local view = require("fennel").view
local integer = G.elements({0, 1, 2, 3, 4})
local wrapped = G.map(function(xs) return {items = xs} end, G.vector(integer))
local function nonzero(value)
  for _, n in ipairs(value.items) do
    if n > 0 then return false end
  end
end
local failure = assert(G.for_all(wrapped, nonzero, {seed = 42, cases = 100}))
assert(#failure.value.items == 1 and failure.value.items[1] == 1,
       "mapped vector failed to shrink to its minimal counterexample")
assert(failure.shrink_attempts > 0 and not failure.exhausted)
local replay = G.for_all(wrapped, nonzero, {seed = 42, cases = 100})
assert(view(replay) == view(failure), "seed did not reproduce failure and shrinking")
assert(not G.for_all(G.tuple({G.constant(nil), G.constant(false), integer}), function(v)
  assert(v[1] == nil and v[2] == false and type(v[3]) == "number")
end))
local tree = G.recursive(G.constant(0), function(child) return G.tuple({child, child}) end)
local function depth(t)
  if type(t) ~= "table" then return 0 end
  return 1 + math.max(depth(t[1]), depth(t[2]))
end
assert(not G.for_all(tree, function(v) assert(depth(v) <= 4) end, {size = 8}))
local throws = assert(G.for_all(integer, function(v)
  assert(v == 0, "deliberate failure")
end))
assert(throws.value == 1 and throws.error:find("deliberate failure"))
for _, bounds in ipairs({{-100, 100}, {-100, -10}, {10, 100}, {4, 4}}) do
  local target = math.max(bounds[1], math.min(0, bounds[2]))
  local failed = assert(G.for_all(G.integer(bounds[1], bounds[2]), function(v)
    assert(v >= bounds[1] and v <= bounds[2] and v == math.floor(v))
    return false
  end))
  assert(failed.value == target, "integer did not shrink within its bounds toward zero")
end
assert(not G.for_all(G.boolean, function(v) assert(type(v) == "boolean") end))
for _, options in ipairs({{cases = 0}, {cases = -1}, {size = -1}, {size = 0.5},
                          {shrinks = -1}, {seed = math.huge}}) do
  assert(not pcall(G.for_all, integer, function() end, options),
         "invalid options silently disabled property coverage")
end
assert(not pcall(G.integer, 2, 1))
assert(not pcall(G.integer, 0, math.huge))
assert(not pcall(G.vector, integer, -1))
print("generator contracts passed")
