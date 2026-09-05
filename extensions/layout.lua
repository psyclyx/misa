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
-- UAX #29 GB9c linkers, kept in lockstep with terminal/width.zig. This is
-- deliberately conservative: only a linker followed by an Indic letter joins.
local virama = {}
for _,cp in ipairs({
  0x094d,0x09cd,0x0a4d,0x0acd,0x0b4d,0x0bcd,0x0c4d,0x0ccd,0x0d3b,0x0d3c,0x0d4d,0x0dca,
  0x0e3a,0x0f84,0x1039,0x103a,0x1714,0x1715,0x1734,0x17d2,0x1a60,0x1b44,0x1baa,0x1bab,
  0x1bf2,0x1bf3,0xa806,0xa8c4,0xa953,0xa9c0,0xaaf6,0xabed,0x10a3f,0x11046,0x11070,
  0x11133,0x11134,0x111c0,0x11235,0x112ea,0x1134d,0x11442,0x114c2,0x115bf,0x115c0,
  0x1163f,0x116b6,0x1172b,0x11839,0x1193d,0x1193e,0x11943,0x119e0,0x11a34,0x11a47,
  0x11a99,0x11c3f,0x11d44,0x11d45,0x11d97,
}) do virama[cp]=true end
local function indic_letter(cp)
  return (cp>=0x0900 and cp<=0x0dff) or (cp>=0x1000 and cp<=0x109f) or
    (cp>=0x1780 and cp<=0x17ff) or (cp>=0xa800 and cp<=0xabff) or (cp>=0x11000 and cp<=0x11dff)
end
local function cell_width(cp) if contains(zero,cp) or virama[cp] then return 0 end; return contains(wide, cp) and 2 or 1 end
local function regional(cp) return cp >= 0x1f1e6 and cp <= 0x1f1ff end
local function cluster(text, at)
  local cp, size = decode(text, at); if not cp then return at, 0 end
  local next_at, cells, emoji, flag, after_virama = at + size, cell_width(cp), false, regional(cp), false
  while next_at <= #text do
    local following, following_size = decode(text, next_at)
    if after_virama and indic_letter(following) and not contains(zero,following) then
      cells,after_virama,next_at=math.max(cells,cell_width(following)),false,next_at+following_size
    elseif following == 0x200d then
      local joined, joined_size = decode(text, next_at + following_size); if not joined then next_at=next_at+following_size; break end
      emoji, next_at, cells = true, next_at + following_size + joined_size, math.max(cells, cell_width(joined))
    elseif virama[following] or contains(zero, following) then
      if following == 0xfe0f or following == 0x20e3 or (following >= 0x1f3fb and following <= 0x1f3ff) then emoji = true end
      if virama[following] then after_virama=true end
      next_at = next_at + following_size
    elseif flag and regional(following) then emoji, next_at = true, next_at + following_size; break
    else break end
  end
  return next_at, emoji and math.max(cells, 2) or cells
end
local function boundary_at_or_before(text, cursor)
  cursor=math.max(0,math.min(#text,math.floor(tonumber(cursor) or 0)))
  local at,previous=1,0
  while at<=#text do local next_at=cluster(text,at); local boundary=next_at-1
    if boundary==cursor then return cursor end
    if boundary>cursor then return previous end
    previous,at=boundary,next_at
  end
  return #text
end
local function previous_boundary(text, cursor)
  cursor=boundary_at_or_before(text,cursor); if cursor==0 then return 0 end
  local at,previous=1,0
  while at<=#text do local next_at=cluster(text,at); local boundary=next_at-1; if boundary>=cursor then return previous end; previous,at=boundary,next_at end
  return previous
end
local function next_boundary(text, cursor)
  cursor=boundary_at_or_before(text,cursor); if cursor>=#text then return #text end
  return cluster(text,cursor+1)-1
end
local function width(text)
  local at, cells = 1, 0; while at <= #text do local next_at, cluster_cells = cluster(text, at); cells, at = cells + cluster_cells, next_at end; return cells
end
local function take(text, columns)
  columns = math.max(0, math.floor(tonumber(columns) or 0)); local at, cells = 1, 0
  while at <= #text do local next_at, cluster_cells = cluster(text, at); local cells_next = cells + cluster_cells
    if cells_next > columns and cells > 0 then break end
    if cells_next > columns and cells == 0 and cluster_cells > 0 then at, cells = next_at, cells_next; break end
    at, cells = next_at, cells_next
  end
  return text:sub(1, at - 1), text:sub(at), cells
end
-- Strict clipping differs from wrapping's take(): a too-wide first grapheme is
-- omitted rather than overflowing. Segmentation is over the complete semantic
-- line, so callers can project the returned byte boundary through style spans.
local function clip(text, columns)
  columns=math.max(0,math.floor(tonumber(columns) or 0)); local at,cells=1,0
  while at<=#text do local next_at,cluster_cells=cluster(text,at)
    if cells+cluster_cells>columns then break end
    cells,at=cells+cluster_cells,next_at
  end
  return text:sub(1,at-1),at-1,cells
end
local function fit(text, columns)
  columns = math.max(0, math.floor(tonumber(columns) or 0)); if columns == 0 then return "" end
  local head, _, cells = take(tostring(text or ""), columns)
  return head .. string.rep(" ", math.max(0, columns - cells))
end
local function clone_spans(spans)
  local result = {}; for _, span in ipairs(spans or {}) do result[#result + 1] = {text=span.text or "",style=span.style,link=span.link} end; return result
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
      if rest == "" and #current == #prefix_spans then current[#current + 1] = {text="",style=source.style,link=source.link} end
      while rest ~= "" do
        if used >= room then flush() end
        local piece, remaining, cells = take(rest, room - used)
        current[#current + 1] = {text=piece,style=source.style,link=source.link}; rest, used = remaining, used + cells
        if rest ~= "" and used >= room then flush() end
      end
    end
    flush()
  end
  return result
end
-- Wrap editable text into physical rows and project its global UTF-8 byte
-- cursor onto the resulting row. Prompt bytes are part of each semantic row,
-- so the native presenter can continue to own byte-to-cell conversion.
local function normalize_newlines(text,cursor)
  text=tostring(text or ""); cursor=math.max(0,math.min(#text,math.floor(tonumber(cursor) or 0)))
  local pieces,mapped,at,bytes={},nil,1,0
  while at<=#text do
    if mapped==nil and at-1>=cursor then mapped=bytes end
    if text:byte(at)==13 then
      local size=text:byte(at+1)==10 and 2 or 1; pieces[#pieces+1]="\n"; bytes=bytes+1; at=at+size
      if mapped==nil and at-1>=cursor then mapped=bytes end
    else
      local _,size=decode(text,at); pieces[#pieces+1]=text:sub(at,at+size-1); bytes=bytes+size; at=at+size
    end
  end
  return table.concat(pieces),mapped or bytes
end
local function wrap_input(text, columns, cursor, prompt, text_style, prompt_style)
  text,cursor=normalize_newlines(text,cursor)
  columns=math.max(1,math.floor(tonumber(columns) or 1)); cursor=boundary_at_or_before(text,cursor)
  -- Preserve two cells for a potentially-wide grapheme whenever possible;
  -- on tiny terminals the prompt yields before editable content does.
  prompt=tostring(prompt or ""); local prompt_room=math.max(0,columns-2)
  prompt=prompt_room==0 and "" or take(prompt,prompt_room); local prompt_cells=width(prompt); local room=math.max(1,columns-prompt_cells)
  local continuation=string.rep(" ",prompt_cells); local rows,mapped={},nil
  local line_start=0
  while true do
    local newline=text:find("\n",line_start+1,true); local line_end=newline and newline-1 or #text
    local value=text:sub(line_start+1,line_end); local segments={}; local at,segment_start,used=1,1,0
    while at<=#value do
      local next_at,cells=cluster(value,at)
      if used>0 and used+cells>room then segments[#segments+1]={first=segment_start,last=at-1}; segment_start,used=at,0 end
      used=used+cells; at=next_at
    end
    segments[#segments+1]={first=segment_start,last=#value}
    for index,segment in ipairs(segments) do
      local prefix=(#rows==0) and prompt or continuation; local piece=value:sub(segment.first,segment.last)
      rows[#rows+1]={spans={{text=prefix,style=prompt_style},{text=piece,style=text_style}}}
      local first_byte=line_start+segment.first-1; local last_byte=line_start+segment.last
      -- Prefer the following physical row at a soft-wrap boundary. This keeps
      -- an insertion cursor off the unusable cell just beyond the right edge.
      if cursor>=first_byte and cursor<=last_byte then mapped={row=#rows,byte=#prefix+(cursor-first_byte)} end
    end
    if not newline then break end
    line_start=newline
  end
  return {lines=rows,cursor=mapped or {row=#rows,byte=#((rows[#rows].spans[1].text or "")..(rows[#rows].spans[2].text or ""))}}
end
local function columns(total, minimum, maximum, gap)
  total, minimum, maximum, gap = math.max(1,math.floor(total)), math.max(1,math.floor(minimum)), math.max(1,math.floor(maximum)), math.max(0,math.floor(gap or 0))
  local count = math.max(1, math.min(maximum, math.floor((total + gap) / (minimum + gap))))
  local usable, widths = math.max(count, total - gap * (count - 1)), {}
  local base, extra = math.floor(usable / count), usable % count
  for index=1,count do widths[index] = base + (index <= extra and 1 or 0) end
  return widths
end
local api = {width=width,take=take,clip=clip,fit=fit,wrap_spans=wrap_spans,wrap_input=wrap_input,columns=columns,cell_width=cell_width,
  boundary_at_or_before=boundary_at_or_before,previous_boundary=previous_boundary,next_boundary=next_boundary}
return {setup=function() misa.layout=api end}
