-- Pure, bounded Markdown parsing. This extension produces semantic data only;
-- terminal layout, colors, and syntax highlighting belong to the view layer.
local defaults={max_source_bytes=262144,max_blocks=4096,max_inlines=16384,max_inline_depth=12,max_link_bytes=4096}

local function integer(value,fallback,minimum,maximum)
  value=tonumber(value); if not value or value%1~=0 then return fallback end
  return math.max(minimum,math.min(maximum,value))
end
local function utf8_prefix(text,limit)
  if #text<=limit then return text,false end
  local last=limit
  while last>0 and text:byte(last+1) and text:byte(last+1)>=0x80 and text:byte(last+1)<0xc0 do last=last-1 end
  return text:sub(1,last),true
end
local function trim(value) return (value:gsub("^%s+",""):gsub("%s+$","")) end
local function marks_copy(marks,mark)
  local result={}; for index,value in ipairs(marks) do result[index]=value end
  if mark then result[#result+1]=mark end; return result
end

local function inline_parser(limits)
  local count=0
  local function parse(text,depth,marks)
    local result,plain={},{}
    local function emit_plain()
      if #plain>0 then result[#result+1]={kind="text",text=table.concat(plain),marks=marks_copy(marks)}; count=count+1; plain={} end
    end
    local function emit(node) emit_plain(); result[#result+1]=node; count=count+1 end
    local at=1
    while at<=#text do
      if count>=limits.max_inlines then plain[#plain+1]=text:sub(at); at=#text+1
      else
        local escaped=text:sub(at,at)=="\\" and text:sub(at+1,at+1) or nil
        if escaped and escaped~="" and escaped:match("[%p]") then plain[#plain+1]=escaped; at=at+2
        else
          local tick_first,tick_last,ticks=text:find("(`+)",at)
          if tick_first~=at then ticks=nil end
          if ticks then
            local close=text:find(ticks,at+#ticks,true)
            if close then emit({kind="code",text=text:sub(at+#ticks,close-1),marks=marks_copy(marks)}); at=close+#ticks else plain[#plain+1]=ticks; at=at+#ticks end
          else
            local label_start,label_end,label=text:find("%[([^%]]-)%]",at)
            if label_start~=at then label_start,label_end=nil,nil end
            local open=label_end and label_end+1 or nil
            if label_start and text:sub(open,open)=="(" then
              local close=text:find(")",open+1,true); local target=close and trim(text:sub(open+1,close-1)) or ""
              local safe=target~="" and #target<=limits.max_link_bytes and not target:find("[%z\1-\31\127-\159]")
              if close and safe then
                emit({kind="link",target=target,children=depth<limits.max_inline_depth and parse(label,depth+1,marks) or {{kind="text",text=label,marks=marks_copy(marks)}}}); at=close+1
              else plain[#plain+1]=text:sub(at,at); at=at+1 end
            else
              local choices={{"***","strong_emphasis"},{"___","strong_emphasis"},{"**","strong"},{"__","strong"},{"~~","strikethrough"},{"*","emphasis"},{"_","emphasis"}}
              local delimiter,mark
              for _,choice in ipairs(choices) do if text:sub(at,at+#choice[1]-1)==choice[1] then delimiter,mark=choice[1],choice[2]; break end end
              if delimiter and depth<limits.max_inline_depth then
                local close=text:find(delimiter,at+#delimiter,true)
                if close and close>at+#delimiter then
                  emit_plain(); local children=parse(text:sub(at+#delimiter,close-1),depth+1,marks_copy(marks,mark))
                  for _,child in ipairs(children) do result[#result+1]=child end
                  at=close+#delimiter
                else plain[#plain+1]=delimiter; at=at+#delimiter end
              else plain[#plain+1]=text:sub(at,at); at=at+1 end
            end
          end
        end
      end
    end
    emit_plain(); return result
  end
  return function(text) count=0; return parse(text,0,{}) end
end

local function split_table(line)
  if not line:find("|",1,true) then return nil end
  local cells,current={},{}; local escaped,in_code=false,false
  for at=1,#line do local char=line:sub(at,at)
    if escaped then current[#current+1]=char; escaped=false
    elseif char=="\\" then escaped=true; current[#current+1]=char
    elseif char=="`" then in_code=not in_code; current[#current+1]=char
    elseif char=="|" and not in_code then cells[#cells+1]=trim(table.concat(current)); current={}
    else current[#current+1]=char end
  end
  cells[#cells+1]=trim(table.concat(current))
  if trim(line):sub(1,1)=="|" then table.remove(cells,1) end
  if trim(line):sub(-1)=="|" then table.remove(cells) end
  if #cells<1 then return nil end; return cells
end
local function table_separator(line)
  local cells=split_table(line); if not cells then return nil end
  local align={}
  for index,cell in ipairs(cells) do
    cell=trim(cell); if not cell:match("^:?-+:?$") or select(2,cell:gsub("-",""))<3 then return nil end
    align[index]=cell:sub(1,1)==":" and (cell:sub(-1)==":" and "center" or "left") or (cell:sub(-1)==":" and "right" or "left")
  end
  return align
end
local function thematic(line)
  local compact=trim(line):gsub("%s",""); if #compact<3 then return false end
  return compact:match("^%*+$") or compact:match("^%-+$") or compact:match("^_+$")
end

local function make_parser(limits)
  local parse_inline=inline_parser(limits)
  return function(value)
    local source=tostring(value or ""):gsub("\r\n","\n"):gsub("\r","\n")
    local truncated; source,truncated=utf8_prefix(source,limits.max_source_bytes)
    local lines={}; for line in (source.."\n"):gmatch("(.-)\n") do lines[#lines+1]=line end
    if source:sub(-1)=="\n" then table.remove(lines) end
    local blocks,index={},1
    local function add(block) if #blocks<limits.max_blocks then blocks[#blocks+1]=block; return true end; truncated=true; return false end
    while index<=#lines and #blocks<limits.max_blocks do
      local raw=lines[index]
      local fence,info=raw:match("^%s*(```+)%s*([^%s`]*)[^`]*$")
      if not fence then fence,info=raw:match("^%s*(~~~+)%s*([^%s~]*)[^~]*$") end
      if fence then
        local body={}; index=index+1
        while index<=#lines do
          local marker=fence:sub(1,1); local closing=lines[index]:match("^%s*([`~]+)%s*$")
          if closing and closing:sub(1,1)==marker and not closing:find("[^"..marker.."]") and #closing>=#fence then break end
          body[#body+1]=lines[index]; index=index+1
        end
        if index<=#lines then index=index+1 end
        add({kind="code_block",language=info:match("^[%w_+#.-]+$") and info:sub(1,64) or "",text=table.concat(body,"\n")})
      else
        local header=split_table(raw); local alignment=index<#lines and table_separator(lines[index+1]) or nil
        if header and alignment and #header==#alignment then
          local rows={{}}; for _,cell in ipairs(header) do rows[1][#rows[1]+1]=parse_inline(cell) end
          index=index+2
          while index<=#lines do local cells=split_table(lines[index]); if not cells then break end
            local row={}; for column=1,#alignment do row[column]=parse_inline(cells[column] or "") end; rows[#rows+1]=row; index=index+1
          end
          add({kind="table",align=alignment,rows=rows})
        else
          local hashes,heading=raw:match("^%s*(#+)%s*(.-)%s*$")
          local quote_body=raw:match("^%s*>%s?(.*)$")
          local indent,bullet,item=raw:match("^(%s*)([-+*])%s+(.+)$")
          local ordered; if not bullet then indent,ordered,item=raw:match("^(%s*)(%d+[.)])%s+(.+)$") end
          if hashes and #hashes<=6 then add({kind="heading",level=#hashes,inlines=parse_inline(heading)}); index=index+1
          elseif thematic(raw) then add({kind="thematic_rule"}); index=index+1
          elseif quote_body then
            local depth=0; local body=raw
            repeat local next_body=body:match("^%s*>%s?(.*)$"); if next_body then depth=depth+1; body=next_body else break end until false
            add({kind="quote",depth=math.min(depth,limits.max_inline_depth),inlines=parse_inline(body)}); index=index+1
          elseif bullet or ordered then
            local checked; local check,item_body=item:match("^%[([ xX])%]%s*(.*)$")
            if check then checked=check~=" "; item=item_body end
            add({kind="list_item",depth=math.min(math.floor(#indent/2)+1,limits.max_inline_depth),ordered=ordered~=nil,number=ordered,checked=checked,inlines=parse_inline(item)}); index=index+1
          elseif raw:match("^%s+") and #blocks>0 and (blocks[#blocks].kind=="list_item" or blocks[#blocks].kind=="list_continuation") then
            local spaces,body=raw:match("^(%s+)(.*)$")
            add({kind="list_continuation",depth=math.min(math.floor(#spaces/2),limits.max_inline_depth),inlines=parse_inline(body)}); index=index+1
          elseif raw=="" then add({kind="blank"}); index=index+1
          else
            local parts={raw}; index=index+1
            while index<=#lines and lines[index]~="" and not lines[index]:match("^%s*(#+)%s*") and not lines[index]:match("^%s*>" ) and not lines[index]:match("^%s*[-+*]%s+") and not lines[index]:match("^%s*%d+[.)]%s+") and not lines[index]:match("^%s*```") and not lines[index]:match("^%s*~~~") and not thematic(lines[index]) do
              if table_separator(lines[index]) then break end
              parts[#parts+1]=trim(lines[index]); index=index+1
            end
            add({kind="paragraph",inlines=parse_inline(table.concat(parts," "))})
          end
        end
      end
    end
    if index<=#lines then truncated=true end
    return {kind="document",blocks=blocks,truncated=truncated or nil}
  end
end

return {setup=function(context)
  assert(misa.markdown==nil,"markdown service already installed")
  local config=type(context.config)=="table" and context.config.markdown or nil; config=type(config)=="table" and config or {}
  local limits={
    max_source_bytes=integer(config.max_source_bytes,defaults.max_source_bytes,1024,1048576),
    max_blocks=integer(config.max_blocks,defaults.max_blocks,16,16384),
    max_inlines=integer(config.max_inlines,defaults.max_inlines,64,65536),
    max_inline_depth=integer(config.max_inline_depth,defaults.max_inline_depth,2,32),
    max_link_bytes=integer(config.max_link_bytes,defaults.max_link_bytes,64,4096),
  }
  misa.markdown={
    parse=make_parser(limits),limits=limits,
    bound=function(value) return utf8_prefix(tostring(value or ""),limits.max_source_bytes) end,
  }
end}
