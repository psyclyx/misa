-- Pure terminal-cell layout primitives. The width tables intentionally mirror
-- src/terminal/width.zig so Lua projections and the native presenter agree.
local zero = {
  {0x0300,0x036f},{0x0483,0x0489},{0x0591,0x05bd},{0x05bf,0x05bf},{0x05c1,0x05c2},{0x05c4,0x05c5},
  {0x0610,0x061a},{0x064b,0x065f},{0x0670,0x0670},{0x06d6,0x06ed},{0x0711,0x0711},{0x0730,0x074a},
  {0x07a6,0x07b0},{0x07eb,0x07f3},{0x0816,0x082d},{0x0859,0x085b},{0x08d3,0x0902},{0x093a,0x093c},
  {0x0941,0x0948},{0x094d,0x094d},{0x0951,0x0957},{0x0962,0x0963},{0x1ab0,0x1aff},{0x1dc0,0x1dff},
  {0x200b,0x200f},{0x202a,0x202e},{0x2060,0x206f},{0x20d0,0x20ff},{0xfe00,0xfe0f},{0xfe20,0xfe2f},
  {0xfeff,0xfeff},{0x1f3fb,0x1f3ff},{0xe0020,0xe007f},{0xe0100,0xe01ef},
}
local wide = {
  {0x1100,0x115f},{0x231a,0x231b},{0x2329,0x232a},{0x23e9,0x23ec},{0x23f0,0x23f0},{0x23f3,0x23f3},
  {0x25fd,0x25fe},{0x2614,0x2615},{0x2648,0x2653},{0x267f,0x267f},{0x2693,0x2693},{0x26a1,0x26a1},
  {0x26aa,0x26ab},{0x26bd,0x26be},{0x26c4,0x26c5},{0x26ce,0x26ce},{0x26d4,0x26d4},{0x26ea,0x26ea},
  {0x26f2,0x26f3},{0x26f5,0x26f5},{0x26fa,0x26fa},{0x26fd,0x26fd},{0x2705,0x2705},{0x270a,0x270b},
  {0x2728,0x2728},{0x274c,0x274c},{0x274e,0x274e},{0x2753,0x2755},{0x2757,0x2757},{0x2795,0x2797},
  {0x27b0,0x27b0},{0x27bf,0x27bf},{0x2b1b,0x2b1c},{0x2b50,0x2b50},{0x2b55,0x2b55},{0x2e80,0x303e},
  {0x3040,0xa4cf},{0xac00,0xd7a3},{0xf900,0xfaff},{0xfe10,0xfe19},{0xfe30,0xfe6f},{0xff00,0xff60},
  {0xffe0,0xffe6},{0x1f004,0x1f004},{0x1f0cf,0x1f0cf},{0x1f18e,0x1f18e},{0x1f191,0x1f19a},
  {0x1f200,0x1f251},{0x1f300,0x1f64f},{0x1f680,0x1f6ff},{0x1f900,0x1f9ff},{0x1fa70,0x1faff},{0x20000,0x3fffd},
}
local function contains(intervals, cp)
  local low, high = 1, #intervals
  while low <= high do local middle = math.floor((low + high) / 2); local range = intervals[middle]
    if cp < range[1] then high = middle - 1 elseif cp > range[2] then low = middle + 1 else return true end
  end
  return false
end
local function decode(text, at)
  local a = text:byte(at); if not a then return nil, 0 end
  local size, cp = 1, a
  if a >= 0xc2 and a <= 0xdf then size, cp = 2, a - 0xc0
  elseif a >= 0xe0 and a <= 0xef then size, cp = 3, a - 0xe0
  elseif a >= 0xf0 and a <= 0xf4 then size, cp = 4, a - 0xf0 end
  for index = at + 1, at + size - 1 do local byte = text:byte(index)
    if not byte or byte < 0x80 or byte >= 0xc0 then return a, 1 end
    cp = cp * 0x40 + byte - 0x80
  end
  return cp, size
end
local function cell_width(cp) if contains(zero, cp) then return 0 end; return contains(wide, cp) and 2 or 1 end
local function width(text)
  local at, cells = 1, 0; while at <= #text do local cp, size = decode(text, at); cells, at = cells + cell_width(cp), at + size end; return cells
end
local function take(text, columns)
  columns = math.max(0, math.floor(tonumber(columns) or 0)); local at, cells = 1, 0
  while at <= #text do local cp, size = decode(text, at); local cells_next = cells + cell_width(cp)
    if cells_next > columns and cells > 0 then break end
    if cells_next > columns and cells == 0 and cell_width(cp) > 0 then at, cells = at + size, cells_next; break end
    at, cells = at + size, cells_next
  end
  -- Keep combining/modifier scalars attached to the preceding base.
  while at <= #text do local cp, size = decode(text, at); if cell_width(cp) ~= 0 then break end; at = at + size end
  return text:sub(1, at - 1), text:sub(at), cells
end
local function fit(text, columns)
  columns = math.max(0, math.floor(tonumber(columns) or 0)); if columns == 0 then return "" end
  local head, _, cells = take(tostring(text or ""), columns)
  return head .. string.rep(" ", math.max(0, columns - cells))
end
local function clone_spans(spans)
  local result = {}; for _, span in ipairs(spans or {}) do result[#result + 1] = {text=span.text or "",style=span.style} end; return result
end
local function wrap_spans(lines, columns, prefix)
  columns = math.max(1, math.floor(tonumber(columns) or 1)); local prefix_spans = type(prefix) == "table" and prefix or {}
  local prefix_cells = 0; for _, span in ipairs(prefix_spans) do prefix_cells = prefix_cells + width(span.text or "") end
  local room, result = math.max(1, columns - prefix_cells), {}
  for _, line in ipairs(lines or {}) do
    local current, used = clone_spans(prefix_spans), 0
    local function flush() result[#result + 1] = {spans=current}; current, used = clone_spans(prefix_spans), 0 end
    for _, source in ipairs(line.spans or {}) do
      local rest = source.text or ""
      if rest == "" and #current == #prefix_spans then current[#current + 1] = {text="",style=source.style} end
      while rest ~= "" do
        if used >= room then flush() end
        local piece, remaining, cells = take(rest, room - used)
        current[#current + 1] = {text=piece,style=source.style}; rest, used = remaining, used + cells
        if rest ~= "" and used >= room then flush() end
      end
    end
    flush()
  end
  return result
end
local function columns(total, minimum, maximum, gap)
  total, minimum, maximum, gap = math.max(1,math.floor(total)), math.max(1,math.floor(minimum)), math.max(1,math.floor(maximum)), math.max(0,math.floor(gap or 0))
  local count = math.max(1, math.min(maximum, math.floor((total + gap) / (minimum + gap))))
  local usable, widths = math.max(count, total - gap * (count - 1)), {}
  local base, extra = math.floor(usable / count), usable % count
  for index=1,count do widths[index] = base + (index <= extra and 1 or 0) end
  return widths
end
local api = {width=width,take=take,fit=fit,wrap_spans=wrap_spans,columns=columns,cell_width=cell_width}
return {setup=function() misa.layout=api end}
