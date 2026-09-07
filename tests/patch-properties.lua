-- Seeded property tests. Patch syntax trees keep the reference evaluator
-- independent of the implementation's opaque delete/replace controls.
local fennel = require("fennel")
local null = {}
local state = fennel.dofile("src/lua_runtime/state.fnl")(null)
local function copy(x)
  if type(x) ~= "table" or x == null then return x end
  local result = {}
  for k, v in pairs(x) do result[k] = copy(v) end
  return result
end
local function equal(a, b)
  if a == b then return true end
  if type(a) ~= "table" or type(b) ~= "table" or a == null or b == null then return false end
  for k, v in pairs(a) do if not equal(v, b[k]) then return false end end
  for k in pairs(b) do if a[k] == nil then return false end end
  return true
end
local function compile(p)
  if p.kind == "delete" then return state.delete end
  if p.kind == "replace" then return state.replace(copy(p.value)) end
  if p.kind == "value" then return copy(p.value) end
  local result = {}
  for k, v in pairs(p.fields) do result[k] = compile(v) end
  return result
end
local function reference(old, p)
  if p.kind == "delete" then return nil end
  if p.kind ~= "merge" then return copy(p.value) end
  local result = type(old) == "table" and old ~= null and copy(old) or {}
  for k, v in pairs(p.fields) do result[k] = reference(result[k], v) end
  -- A merge containing only ineffective changes is a no-op even on a scalar.
  if next(result) == nil and not (type(old) == "table" and old ~= null) then return old end
  return result
end
local function sharing(old, new)
  if type(old) ~= "table" or old == null then return end
  if equal(old, new) then assert(old == new, "equal branch lost identity") end
  if type(new) == "table" and new ~= null then
    for k, v in pairs(old) do sharing(v, new[k]) end
  end
end
local function check(old, ast)
  local before, syntax_before = copy(old), copy(ast)
  local patch = compile(ast)
  local result = state.patch(old, patch)
  assert(equal(result, reference(old, ast)), "reference mismatch")
  assert(equal(old, before), "input state mutated")
  assert(equal(ast, syntax_before), "patch syntax mutated")
  sharing(old, result)
  assert(state.patch(result, patch) == result, "patch is not identity-idempotent")
  assert(state.patch(old, {}) == old, "empty patch changed identity")
  -- Disjoint updates commute, including deletion and replacement.
  local a = {kind = "merge", fields = {a = ast.fields.a}}
  local b = {kind = "merge", fields = {b = ast.fields.b}}
  assert(equal(state.patch(state.patch(old, compile(a)), compile(b)),
               state.patch(state.patch(old, compile(b)), compile(a))),
         "disjoint patches did not commute")
end
local G = require("tests.generators")
local function fields(generator)
  return G.map(function(entries)
    local result = {}
    for _, entry in ipairs(entries) do result[entry[1]] = entry[2] end
    return result
  end, G.vector(G.tuple({G.elements({"a", "b", "c"}), generator})))
end
local scalar = G.one_of({G.boolean, G.integer(-100, 100), G.elements({"", "text", null})})
local data = G.recursive(scalar, function(child)
  return G.one_of({fields(child), G.vector(child)})
end)
local function tagged(kind, key, generator)
  return G.map(function(v) return {kind = kind, [key] = v} end, generator)
end
local leaf = G.one_of({
  G.constant({kind = "delete"}),
  tagged("value", "value", scalar),
  tagged("replace", "value", G.one_of({data, G.constant(nil)})),
  tagged("value", "value", G.vector(data, 1)),
})
local patch = G.recursive(leaf, function(child)
  return tagged("merge", "fields", fields(child))
end)
local inputs = G.tuple({fields(data), tagged("merge", "fields", fields(patch))})
local failure = G.for_all(inputs, function(pair) check(pair[1], pair[2]) end, {
  seed = tonumber(os.getenv("MISA_PROPERTY_SEED")) or 1729, cases = 3000, size = 6,
})
assert(not failure, failure and fennel.view(failure))
-- Retained snapshots must survive subsequent updates. Shrinking removes whole
-- operations as well as simplifying their payloads, without a custom runner.
local sequence_failure = G.for_all(
  G.tuple({fields(data), G.vector(tagged("merge", "fields", fields(patch)))}),
  function(input)
    local actual, expected = input[1], copy(input[1])
    local retained = {}
    for _, operation in ipairs(input[2]) do
      retained[#retained + 1] = {actual, copy(actual)}
      check(actual, operation)
      actual = state.patch(actual, compile(operation))
      expected = reference(expected, operation)
      assert(equal(actual, expected), "sequence reference mismatch")
      for _, snapshot in ipairs(retained) do
        assert(equal(snapshot[1], snapshot[2]), "retained snapshot mutated")
      end
    end
  end,
  {seed = tonumber(os.getenv("MISA_PROPERTY_SEED")) or 1729, cases = 500, size = 6})
assert(not sequence_failure, sequence_failure and fennel.view(sequence_failure))
print("patch properties passed")
