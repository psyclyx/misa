-- Shared orderless fuzzy matching for every choice surface.
local function words(value)
  local result = {}
  for word in value:lower():gmatch("%S+") do result[#result + 1] = word end
  return result
end

local function subsequence(needle, haystack)
  local at, first, previous, gaps = 1, nil, nil, 0
  for index = 1, #haystack do
    if haystack:sub(index, index) == needle:sub(at, at) then
      first = first or index
      if previous then gaps = gaps + index - previous - 1 end
      previous, at = index, at + 1
      if at > #needle then return first + gaps * 2 end
    end
  end
end

local function score(query, text)
  local total, lower = 0, text:lower()
  for _, word in ipairs(words(query)) do
    local exact = lower:find(word, 1, true)
    local part = exact and (exact - 1) or subsequence(word, lower)
    if not part then return nil end
    total = total + part
  end
  return total
end

return {
  setup = function()
    misa.fuzzy_score = score
    misa.fuzzy_choices = function(source, query, text)
      local ranked = {}
      for ordinal, item in ipairs(source) do
        local searchable = text and text(item) or (item.value .. " " .. (item.label or "") .. " " .. (item.description or ""))
        local rank = query == "" and 0 or score(query, searchable)
        if rank then ranked[#ranked + 1] = { item = item, score = rank, ordinal = ordinal } end
      end
      table.sort(ranked, function(left, right)
        if left.score ~= right.score then return left.score < right.score end
        local lv, rv = left.item.value or left.item.name or "", right.item.value or right.item.name or ""
        if lv ~= rv then return lv < rv end
        return left.ordinal < right.ordinal
      end)
      local result = {}
      for _, value in ipairs(ranked) do result[#result + 1] = value.item end
      return result
    end
  end,
}
