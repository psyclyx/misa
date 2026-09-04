-- Provider-neutral JSON values used at Lua protocol boundaries.
local function utf8(codepoint)
  if codepoint <= 0x7f then return string.char(codepoint) end
  if codepoint <= 0x7ff then return string.char(0xc0 + math.floor(codepoint / 0x40), 0x80 + codepoint % 0x40) end
  if codepoint <= 0xffff then return string.char(0xe0 + math.floor(codepoint / 0x1000), 0x80 + math.floor(codepoint / 0x40) % 0x40, 0x80 + codepoint % 0x40) end
  return string.char(0xf0 + math.floor(codepoint / 0x40000), 0x80 + math.floor(codepoint / 0x1000) % 0x40, 0x80 + math.floor(codepoint / 0x40) % 0x40, 0x80 + codepoint % 0x40)
end

local function decode(source)
  assert(type(source) == "string", "JSON source must be a string")
  local at = 1
  local function fail(message) error(message .. " at byte " .. tostring(at), 0) end
  local function whitespace() while source:sub(at, at):match("%s") do at = at + 1 end end
  local value
  local function string_value()
    if source:sub(at, at) ~= '"' then fail("expected JSON string") end
    at = at + 1; local parts, start = {}, at
    while at <= #source do
      local byte = source:byte(at)
      if byte == 34 then parts[#parts + 1] = source:sub(start, at - 1); at = at + 1; return table.concat(parts) end
      if byte < 32 then fail("unescaped control character in JSON string") end
      if byte == 92 then
        parts[#parts + 1] = source:sub(start, at - 1); at = at + 1
        local escape = source:sub(at, at)
        local simple = { ['"']='"', ['\\']='\\', ['/']='/', b='\b', f='\f', n='\n', r='\r', t='\t' }
        if simple[escape] then parts[#parts + 1] = simple[escape]; at = at + 1
        elseif escape == "u" then
          local hex = source:sub(at + 1, at + 4); if not hex:match("^%x%x%x%x$") then fail("invalid JSON unicode escape") end
          local codepoint = tonumber(hex, 16); at = at + 5
          if codepoint >= 0xd800 and codepoint <= 0xdbff then
            local low = source:sub(at, at + 5); local low_hex = low:match("^\\u(%x%x%x%x)$")
            local low_code = low_hex and tonumber(low_hex, 16) or nil
            if not low_code or low_code < 0xdc00 or low_code > 0xdfff then fail("invalid JSON surrogate pair") end
            codepoint = 0x10000 + (codepoint - 0xd800) * 0x400 + low_code - 0xdc00; at = at + 6
          elseif codepoint >= 0xdc00 and codepoint <= 0xdfff then fail("invalid JSON surrogate") end
          parts[#parts + 1] = utf8(codepoint)
        else fail("invalid JSON escape") end
        start = at
      else at = at + 1 end
    end
    fail("unterminated JSON string")
  end
  local function array_value()
    at = at + 1; whitespace(); local result = {}
    if source:sub(at, at) == "]" then at = at + 1; return result end
    while true do
      result[#result + 1] = value(); whitespace()
      local delimiter = source:sub(at, at); at = at + 1
      if delimiter == "]" then return result end
      if delimiter ~= "," then fail("expected ',' or ']' in JSON array") end
      whitespace()
    end
  end
  local function object_value()
    at = at + 1; whitespace(); local result = {}
    if source:sub(at, at) == "}" then at = at + 1; return result end
    while true do
      local key = string_value(); whitespace()
      if source:sub(at, at) ~= ":" then fail("expected ':' in JSON object") end
      at = at + 1; whitespace(); result[key] = value(); whitespace()
      local delimiter = source:sub(at, at); at = at + 1
      if delimiter == "}" then return result end
      if delimiter ~= "," then fail("expected ',' or '}' in JSON object") end
      whitespace()
    end
  end
  value = function()
    whitespace(); local first = source:sub(at, at)
    if first == '"' then return string_value() end
    if first == "[" then return array_value() end
    if first == "{" then return object_value() end
    for literal, decoded in pairs({ ["true"]=true, ["false"]=false, ["null"]=misa.json_null }) do
      if source:sub(at, at + #literal - 1) == literal then at = at + #literal; return decoded end
    end
    local token = source:sub(at):match("^([^,%]%}%s]+)")
    local integer = token and (token:match("^-?0$") or token:match("^-?[1-9]%d*$"))
    local decimal = token and (token:match("^-?0%.%d+$") or token:match("^-?[1-9]%d*%.%d+$"))
    local exponent = token and (token:match("^-?0[eE][+-]?%d+$") or token:match("^-?[1-9]%d*[eE][+-]?%d+$") or token:match("^-?0%.%d+[eE][+-]?%d+$") or token:match("^-?[1-9]%d*%.%d+[eE][+-]?%d+$"))
    if integer or decimal or exponent then at = at + #token; return tonumber(token) end
    fail("invalid JSON value")
  end
  local result = value(); whitespace(); if at <= #source then fail("trailing JSON data") end
  return result
end

local escapes = { ['"']='\\"', ['\\']='\\\\', ['\b']='\\b', ['\f']='\\f', ['\n']='\\n', ['\r']='\\r', ['\t']='\\t' }
local function encode(root)
  local active = {}
  local function visit(value)
    if value == misa.json_null then return "null" end
    local kind = type(value)
    if kind == "string" then return '"' .. value:gsub('[%z\1-\31\\"]', function(char) return escapes[char] or string.format("\\u%04x", char:byte()) end) .. '"' end
    if kind == "boolean" then return value and "true" or "false" end
    if kind == "number" then assert(value == value and value ~= math.huge and value ~= -math.huge, "JSON numbers must be finite"); return tostring(value) end
    assert(kind == "table", "unsupported JSON value: " .. kind)
    assert(not active[value], "cyclic JSON value"); active[value] = true
    local count, maximum, array = 0, 0, true
    for key in pairs(value) do
      count = count + 1
      if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then array = false else maximum = math.max(maximum, key) end
    end
    array = array and count > 0 and maximum == count
    local parts = {}
    if array then for index = 1, count do parts[index] = visit(value[index]) end
    else
      local keys = {}; for key in pairs(value) do assert(type(key) == "string", "JSON object keys must be strings"); keys[#keys + 1] = key end
      table.sort(keys); for _, key in ipairs(keys) do parts[#parts + 1] = visit(key) .. ":" .. visit(value[key]) end
    end
    active[value] = nil
    return (array and "[" or "{") .. table.concat(parts, ",") .. (array and "]" or "}")
  end
  return visit(root)
end

return { setup = function()
  assert(misa.json == nil, "JSON utility already registered")
  misa.json = { decode = decode, encode = encode }
end }
