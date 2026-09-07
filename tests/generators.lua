-- A generator is (random, size) -> {value, shrinks}. Shrinks are lazy sample
-- trees: mapping a generator maps its shrink tree, preserving valid shapes.
local G = {}
local function sample(value, shrinks)
  return {value = value, shrinks = shrinks or function() return {} end}
end
function G.constant(value)
  return function() return sample(value) end
end
function G.elements(values)
  assert(#values > 0)
  local function at(i)
    return sample(values[i], function()
      local smaller = {}
      for j = 1, i - 1 do smaller[j] = at(j) end
      return smaller
    end)
  end
  return function(random) return at(random(#values)) end
end
local function mapped(f, node)
  return sample(f(node.value), function()
    local result = {}
    for i, child in ipairs(node.shrinks()) do result[i] = mapped(f, child) end
    return result
  end)
end
function G.map(f, generator)
  return function(random, size) return mapped(f, generator(random, size)) end
end
function G.one_of(generators)
  assert(#generators > 0)
  return function(random, size) return generators[random(#generators)](random, size) end
end
local function collection(nodes, removable, minimum)
  local values = {}
  for i, node in ipairs(nodes) do values[i] = node.value end
  return sample(values, function()
    local result = {}
    local function candidate(index, replacement)
      local smaller = {}
      for i, node in ipairs(nodes) do
        if i ~= index then smaller[#smaller + 1] = node
        elseif replacement then smaller[#smaller + 1] = replacement end
      end
      result[#result + 1] = collection(smaller, removable, minimum)
    end
    if removable and #nodes > minimum then
      for i = 1, #nodes do candidate(i) end
    end
    for i, node in ipairs(nodes) do
      for _, child in ipairs(node.shrinks()) do candidate(i, child) end
    end
    return result
  end)
end
function G.tuple(generators)
  return function(random, size)
    local nodes = {}
    for i, generator in ipairs(generators) do nodes[i] = generator(random, size) end
    return collection(nodes, false, #nodes)
  end
end
function G.vector(generator, minimum)
  minimum = minimum or 0
  return function(random, size)
    local nodes = {}
    for i = 1, minimum + random(size + 1) - 1 do nodes[i] = generator(random, size) end
    return collection(nodes, true, minimum)
  end
end
function G.recursive(leaf, branch)
  local function generate(random, size)
    if size <= 0 then return leaf(random, 0) end
    local child = function(r) return generate(r, math.floor(size / 2)) end
    return G.one_of({leaf, branch(child)})(random, size)
  end
  return generate
end
function G.for_all(generator, property, options)
  options = options or {}
  local seed = options.seed or 1729
  assert(seed >= 1 and seed < 2147483647 and seed == math.floor(seed), "invalid property seed")
  local current = seed
  local function random(n)
    current = current * 16807 % 2147483647
    return current % n + 1
  end
  local function failure(node)
    local ok, result = pcall(property, node.value)
    if not ok then return tostring(result) end
    if result == false then return "property returned false" end
  end
  for case = 1, options.cases or 100 do
    local node = generator(random, (case - 1) % ((options.size or 8) + 1))
    local err = failure(node)
    if err then
      local attempts, budget = 0, options.shrinks or 1000
      while attempts < budget do
        local smaller
        for _, candidate in ipairs(node.shrinks()) do
          if attempts >= budget then break end
          attempts = attempts + 1
          if failure(candidate) == err then smaller = candidate; break end
        end
        if not smaller then break end
        node = smaller
      end
      return {seed = seed, case = case, value = node.value, error = err,
              shrink_attempts = attempts, exhausted = attempts == budget}
    end
  end
end
return G
