-- Semantic terminal flow for documents produced by the markdown extension.
-- It owns layout, but only emits theme tokens; theme resolution remains at the
-- component registry boundary.
local syntax_classes={comment=true,string=true,number=true,keyword=true,type=true,["function"]=true,constant=true,variable=true,property=true,tag=true,attribute=true,operator=true,punctuation=true,escape=true,embedded=true}
local heading_marks={"markdown.heading.1","markdown.heading.2","markdown.heading.3","markdown.heading.4","markdown.heading.5","markdown.heading.6"}
local heading_prefix={"█ ","▌ ","▸ ","▪ ","· ","· "}

local function compose(base,...)
  local result={}; if type(base)=="table" then for _,token in ipairs(base) do result[#result+1]=token end else result[1]=base or "plain" end
  for index=1,select("#",...) do local token=select(index,...); if token then result[#result+1]=token end end
  return result
end
local function span(text,style,link) return {text=text,style=style,link=link} end
local function clone_prefix(prefix)
  local result={}; for _,item in ipairs(prefix or {}) do result[#result+1]=span(item.text,item.style,item.link) end; return result
end
local function inline_spans(nodes,base,extra)
  local result={}
  local function visit(items,link)
    for _,node in ipairs(items or {}) do
      if node.kind=="link" then visit(node.children,node.target)
      else
        local style=compose(base,extra); for _,mark in ipairs(node.marks or {}) do
          if mark=="strong" then style[#style+1]="bold"
          elseif mark=="emphasis" then style[#style+1]="italic"
          elseif mark=="strong_emphasis" then style[#style+1]="bold"; style[#style+1]="italic"
          elseif mark=="strikethrough" then style[#style+1]="strikethrough" end
        end
        if node.kind=="code" then style[#style+1]="code" end
        if link then style[#style+1]="link" end
        result[#result+1]=span(node.text or "",style,link)
      end
    end
  end
  visit(nodes,nil); if #result==0 then result[1]=span("",compose(base,extra)) end; return result
end
local function prefix_width(prefix) local width=0; for _,item in ipairs(prefix or {}) do width=width+misa.layout.width(item.text or "") end; return width end
local function flow(spans,columns,first_prefix,rest_prefix)
  columns=math.max(1,columns); first_prefix=first_prefix or {}; rest_prefix=rest_prefix or first_prefix
  local result,current,used={},clone_prefix(first_prefix),prefix_width(first_prefix)
  local first=true
  local function flush()
    result[#result+1]={spans=current}; first=false; current=clone_prefix(rest_prefix); used=prefix_width(rest_prefix)
  end
  for _,source in ipairs(spans) do
    local rest=source.text or ""
    if rest=="" and #current==#(first and first_prefix or rest_prefix) then current[#current+1]=span("",source.style,source.link) end
    while rest~="" do
      local room=math.max(1,columns-used); local piece,remaining,cells=misa.layout.take(rest,room)
      current[#current+1]=span(piece,source.style,source.link); rest,used=remaining,used+cells
      if rest~="" then flush() end
    end
  end
  flush(); return result
end
local function append(target,lines) for _,line in ipairs(lines) do target[#target+1]=line end end
local function pad_spans(spans,width,alignment,base)
  local used=0; for _,item in ipairs(spans) do used=used+misa.layout.width(item.text or "") end
  local gap=math.max(0,width-used); local left=alignment=="right" and gap or (alignment=="center" and math.floor(gap/2) or 0)
  local right=gap-left; local result={}
  if left>0 then result[#result+1]=span(string.rep(" ",left),base) end
  for _,item in ipairs(spans) do result[#result+1]=item end
  if right>0 then result[#result+1]=span(string.rep(" ",right),base) end
  return result
end
local function cell_lines(nodes,width,base,header)
  return flow(inline_spans(nodes,base,header and "markdown.table.header" or nil),math.max(1,width),{}, {})
end
local function allocate_columns(table_block,available)
  local count=#table_block.align; local widths={}; for column=1,count do widths[column]=3 end
  for _,row in ipairs(table_block.rows) do for column=1,count do
    local natural=0; for _,item in ipairs(inline_spans(row[column] or {},"plain")) do natural=natural+misa.layout.width(item.text or "") end
    widths[column]=math.max(widths[column],math.min(40,natural))
  end end
  local room=math.max(count,available-(count+1)-2*count)
  local total=0; for _,width in ipairs(widths) do total=total+width end
  while total>room do
    local widest=1; for column=2,count do if widths[column]>widths[widest] then widest=column end end
    if widths[widest]<=1 then break end; widths[widest]=widths[widest]-1; total=total-1
  end
  while total<room do
    local grew=false; for column=1,count do if total<room and widths[column]<40 then widths[column]=widths[column]+1; total=total+1; grew=true end end
    if not grew then break end
  end
  return widths
end
local function border(widths,left,middle,right,base)
  local pieces={left}; for index,width in ipairs(widths) do pieces[#pieces+1]=string.rep("─",width+2); pieces[#pieces+1]=index==#widths and right or middle end
  return {spans={span(table.concat(pieces),compose(base,"markdown.table.border"))}}
end
local function render_table(block,columns,base)
  local widths=allocate_columns(block,columns); local result={border(widths,"┌","┬","┐",base)}
  for row_index,row in ipairs(block.rows) do
    local cells,height={},1
    for column,width in ipairs(widths) do cells[column]=cell_lines(row[column] or {},width,base,row_index==1); height=math.max(height,#cells[column]) end
    for line_index=1,height do local pieces={span("│",compose(base,"markdown.table.border"))}
      for column,width in ipairs(widths) do
        pieces[#pieces+1]=span(" ",base); local content=(cells[column][line_index] or {spans={span("",base)}}).spans
        for _,item in ipairs(pad_spans(content,width,block.align[column],base)) do pieces[#pieces+1]=item end
        pieces[#pieces+1]=span(" │",compose(base,"markdown.table.border"))
      end
      result[#result+1]={spans=pieces}
    end
    if row_index<#block.rows then result[#result+1]=border(widths,"├","┼","┤",base) end
  end
  result[#result+1]=border(widths,"└","┴","┘",base); return result
end
local function highlighted_lines(block,base)
  local source=block.text or ""; local captures={}
  if block.language~="" and misa.syntax and misa.syntax.highlight then local ok,value=pcall(misa.syntax.highlight,block.language,source); if ok and type(value)=="table" then captures=value end end
  local spans,at={},1
  for _,capture in ipairs(captures) do
    local first=(tonumber(capture.start_byte) or 0)+1; local after=(tonumber(capture.end_byte) or 0)+1; local class=capture.capture
    if syntax_classes[class] and first>=at and after>first and after<=#source+1 then
      if first>at then spans[#spans+1]=span(source:sub(at,first-1),compose(base,"code")) end
      spans[#spans+1]=span(source:sub(first,after-1),compose(base,"code","syntax."..class)); at=after
    end
  end
  if at<=#source then spans[#spans+1]=span(source:sub(at),compose(base,"code")) end
  if #spans==0 then spans[1]=span(source,compose(base,"code")) end
  local lines={{spans={}}}
  for _,item in ipairs(spans) do
    local rest=item.text
    while true do local newline=rest:find("\n",1,true)
      if not newline then lines[#lines].spans[#lines[#lines].spans+1]=span(rest,item.style,item.link); break end
      lines[#lines].spans[#lines[#lines].spans+1]=span(rest:sub(1,newline-1),item.style,item.link); lines[#lines+1]={spans={}}; rest=rest:sub(newline+1)
    end
  end
  return lines
end
local function code_block(block,columns,base)
  local language=block.language~="" and block.language or "plain"; local result={}
  local label=misa.layout.take(language,math.max(1,columns-4)); local label_width=misa.layout.width(label)
  result[1]={spans={span("╭─ "..label.." ",compose(base,"markdown.code.label")),span(string.rep("─",math.max(0,columns-label_width-4)),compose(base,"markdown.code.border"))}}
  for _,line in ipairs(highlighted_lines(block,base)) do append(result,flow(line.spans,columns,{span("│ ",compose(base,"markdown.code.border"))},{span("│ ",compose(base,"markdown.code.border"))})) end
  result[#result+1]={spans={span("╰"..string.rep("─",math.max(0,columns-1)),compose(base,"markdown.code.border"))}}; return result
end

local function render(document,options)
  options=options or {}; local base=options.base or "plain"; local columns=math.max(4,math.min(512,math.floor(tonumber(options.columns) or 80))); local result={}
  for _,block in ipairs(document.blocks or {}) do
    if block.kind=="blank" then result[#result+1]={spans={span("",base)}}
    elseif block.kind=="paragraph" then append(result,flow(inline_spans(block.inlines,base),columns))
    elseif block.kind=="heading" then local token=heading_marks[block.level] or heading_marks[6]; local prefix={span(heading_prefix[block.level] or "· ",compose(base,token))}; append(result,flow(inline_spans(block.inlines,base,token),columns,prefix,{span("  ",base)}))
    elseif block.kind=="quote" then local rails={}; for _=1,math.max(1,block.depth or 1) do rails[#rails+1]=span("▏ ",compose(base,"quote")) end; append(result,flow(inline_spans(block.inlines,base,"quote"),columns,rails,rails))
    elseif block.kind=="list_item" then
      local indent=string.rep("  ",math.max(0,(block.depth or 1)-1)); local marker=block.checked~=nil and (block.checked and "☑ " or "☐ ") or (block.ordered and tostring(block.number or "1.").." " or "• ")
      local first={span(indent,base),span(marker,compose(base,"markdown.list.marker"))}; local rest={span(indent..string.rep(" ",misa.layout.width(marker)),base)}
      append(result,flow(inline_spans(block.inlines,base),columns,first,rest))
    elseif block.kind=="list_continuation" then local prefix={span(string.rep("  ",math.max(1,block.depth or 1)),base)}; append(result,flow(inline_spans(block.inlines,base),columns,prefix,prefix))
    elseif block.kind=="thematic_rule" then result[#result+1]={spans={span(string.rep("─",columns),compose(base,"markdown.rule"))}}
    elseif block.kind=="table" then append(result,render_table(block,columns,base))
    elseif block.kind=="code_block" then append(result,code_block(block,columns,base)) end
  end
  if document.truncated then result[#result+1]={spans={span("… truncated",compose(base,"dim"))}} end
  if #result==0 then result[1]={spans={span("",base)}} end; return result
end
local function plain(text,base)
  local truncated; text,truncated=misa.markdown.bound(text); text=text:gsub("\r\n","\n"):gsub("\r","\n"); if text:sub(-1)=="\n" then text=text:sub(1,-2) end
  local result={}; for line in (text.."\n"):gmatch("(.-)\n") do result[#result+1]={spans={span(line,base)}} end
  if truncated then result[#result+1]={spans={span("… truncated",compose(base,"dim"))}} end
  if #result==0 then result[1]={spans={span("",base)}} end; return result
end
return {setup=function()
  assert(misa.markdown and type(misa.markdown.parse)=="function","component.markdown requires markdown")
  assert(misa.layout,"component.markdown requires layout")
  misa.markdown_view={render=render,plain=plain}
end}
