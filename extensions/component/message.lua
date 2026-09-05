-- Default transcript visuals: semantic data in, semantic lines out.
local function span(text, style, link) return {text=text,style=style,link=link} end
local function composed(base,modifier)
  if not modifier then return base end
  if type(base)~="table" then return {base,modifier} end
  local result={}; for _,token in ipairs(base) do result[#result+1]=token end; result[#result+1]=modifier; return result
end
local function plain_lines(text, style)
  local result={}; text=tostring(text or ""):gsub("\r\n","\n"):gsub("\r","\n"); if text:sub(-1)=="\n" then text=text:sub(1,-2) end
  if text=="" then return {{spans={span("",style)}}} end
  for line in (text.."\n"):gmatch("(.-)\n") do result[#result+1]={spans={span(line,style)}} end; return result
end
local function inline(text, base)
  local result,at={},1
  local function add(value,modifier,link) if value~="" then result[#result+1]=span(value,composed(base,modifier),link) end end
  while at<=#text do
    local first,last,value=text:find("%*%*(.-)%*%*",at); if first~=at then first=nil end
    if first then add(value,"bold"); at=last+1 else
      first,last,value=text:find("~~(.-)~~",at); if first~=at then first=nil end
      if first then add(value,"strikethrough"); at=last+1 else
        first,last,value=text:find("`([^`]+)`",at); if first~=at then first=nil end
        if first then add(value,"code"); at=last+1 else
          local label,url; first,last,label,url=text:find("%[([^%]]+)%]%(([^%)]+)%)",at); if first~=at then first=nil end
          if first then add(label,"link",url); add(" ("..url..")","dim",url); at=last+1 else
            first,last,value=text:find("%*([^*]+)%*",at); if first~=at then first=nil end
            if not first then first,last,value=text:find("_([^_]+)_",at); if first~=at then first=nil end end
            if first then add(value,"italic"); at=last+1 else
              local next_mark=#text+1; for _,mark in ipairs({"%*%*","~~","`","%[","%*","_"}) do local found=text:find(mark,at+1); if found and found<next_mark then next_mark=found end end
              add(text:sub(at,next_mark-1)); at=next_mark
            end
          end
        end
      end
    end
  end
  if #result==0 then result[1]=span("",base) end; return result
end
local function markdown_lines(text,base,enabled)
  if not enabled then return plain_lines(text,base) end
  local result,fenced={},false; text=tostring(text or ""):gsub("\r\n","\n"):gsub("\r","\n"); if text:sub(-1)=="\n" then text=text:sub(1,-2) end
  for raw in (text.."\n"):gmatch("(.-)\n") do
    if raw:match("^%s*```") then fenced=not fenced
    elseif fenced then result[#result+1]={spans={span(raw,composed(base,"code"))}}
    else
      local style,content,prefix,prefix_style=base,raw,nil,nil; local hashes,heading=raw:match("^(#+)%s+(.+)$")
      if hashes then style,content=composed(base,"bold"),heading else local quote=raw:match("^%s*>%s?(.*)$")
        if quote then style,content,prefix,prefix_style=composed(base,"quote"),quote,"▏ ","quote" else local bullet,item=raw:match("^%s*([-*+])%s+(.+)$")
          if bullet then content,prefix,prefix_style=item,"• ","accent" else local number,ordered=raw:match("^%s*(%d+%.)%s+(.+)$"); if number then content,prefix,prefix_style=ordered,number.." ","accent" end end
        end
      end
      local spans=inline(content,style); if prefix then table.insert(spans,1,span(prefix,composed(base,prefix_style))) end; result[#result+1]={spans=spans}
    end
  end
  if #result==0 then result[1]={spans={span("",base)}} end; return result
end
local function title(model,label,style)
  local parts={span(label,composed(style or "plain","bold"))}
  if model.timestamp then parts[#parts+1]=span("  "..tostring(model.timestamp),"dim") end
  if model.streaming then parts[#parts+1]=span("  streaming","accent") end
  if model.interrupted then parts[#parts+1]=span("  interrupted","error") end
  if type(model.tokens_per_second)=="number" then parts[#parts+1]=span("  "..string.format("%.1f",model.tokens_per_second).." tok/s","dim") end
  return {spans=parts}
end
local function rail(model) return assert(model.rail,"message model requires a semantic rail token") end
local function message(model,context,style,label)
  if not context.interactive then return plain_lines(model.text,style) end
  local rendered=markdown_lines(model.text,style,context.markdown~=false)
  rendered=misa.layout.wrap_spans(rendered,context.columns,{{text="┃ ",style=rail(model)}})
  if label then table.insert(rendered,1,title(model,label,style)) end
  return rendered
end
local function titled(model,label,text,style)
  return {lines={title(model,label,style),{spans={span("┃ ",rail(model)),span(text,style)}}}}
end
return {setup=function()
  assert(misa.layout,"component.message requires layout")
  local function reg(role,render) misa.reg_component("default."..role,{render=render}) end
  reg("transcript.user",function(model,context) return {lines=context.interactive and message(model,context,"user","You") or {}} end)
  reg("transcript.assistant",function(model,context) return {lines=message(model,context,"assistant","Assistant")} end)
  reg("transcript.thinking",function(model,context) return {lines=message(model,context,"thinking","Thinking")} end)
  reg("transcript.thinking_collapsed",function(model) return titled(model,"Thinking",tostring(model.summary or "summary"),"thinking") end)
  reg("transcript.tool_call",function(model) return titled(model,"Tool · "..tostring(model.name or "tool"),tostring(model.detail or "summary"),"tool") end)
  reg("transcript.tool_result",function(model,context) if model.collapsed then return titled(model,model.is_error and "Tool error" or "Tool result","summary",model.is_error and "error" or "tool") end return {lines=message(model,context,model.is_error and "error" or "tool",model.is_error and "Tool error" or "Tool result")} end)
  reg("transcript.harness",function(model,context) return {lines=message(model,context,model.level=="error" and "error" or "plain",nil)} end)
end}
