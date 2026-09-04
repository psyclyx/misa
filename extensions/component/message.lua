-- Default transcript visuals: semantic data in, semantic lines out.
local function span(text, style) return {text=text,style=style} end
local function plain_lines(text, style)
  local result={}; text=tostring(text or ""):gsub("\r\n","\n"):gsub("\r","\n"); if text:sub(-1)=="\n" then text=text:sub(1,-2) end
  if text=="" then return {{spans={span("",style)}}} end
  for line in (text.."\n"):gmatch("(.-)\n") do result[#result+1]={spans={span(line,style)}} end; return result
end
local function inline(text, base)
  local result,at={},1; local function add(value,style) if value~="" then result[#result+1]=span(value,style or base) end end
  while at<=#text do
    local first,last,value=text:find("%*%*(.-)%*%*",at); if first~=at then first=nil end
    if first then add(value,"bold"); at=last+1 else
      first,last,value=text:find("`([^`]+)`",at); if first~=at then first=nil end
      if first then add(value,"accent"); at=last+1 else
        local label,url; first,last,label,url=text:find("%[([^%]]+)%]%(([^%)]+)%)",at); if first~=at then first=nil end
        if first then add(label,"accent"); add(" ("..url..")","dim"); at=last+1 else
          first,last,value=text:find("%*([^*]+)%*",at); if first~=at then first=nil end
          if not first then first,last,value=text:find("_([^_]+)_",at); if first~=at then first=nil end end
          if first then add(value,"bold"); at=last+1 else
            local next_mark=#text+1; for _,mark in ipairs({"%*%*","`","%[","%*","_"}) do local found=text:find(mark,at+1); if found and found<next_mark then next_mark=found end end
            add(text:sub(at,next_mark-1)); at=next_mark
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
    elseif fenced then result[#result+1]={spans={span(raw,"accent")}}
    else
      local style,content,prefix=base,raw,nil; local hashes,heading=raw:match("^(#+)%s+(.+)$")
      if hashes then style,content="bold",heading else local quote=raw:match("^%s*>%s?(.*)$")
        if quote then style,content,prefix="dim",quote,"│ " else local bullet,item=raw:match("^%s*([-*+])%s+(.+)$")
          if bullet then content,prefix=item,"• " else local number,ordered=raw:match("^%s*(%d+%.)%s+(.+)$"); if number then content,prefix=ordered,number.." " end end
        end
      end
      local spans=inline(content,style); if prefix then table.insert(spans,1,span(prefix,style=="dim" and "dim" or "accent")) end; result[#result+1]={spans=spans}
    end
  end
  if #result==0 then result[1]={spans={span("",base)}} end; return result
end
local function message(model,context,style,label)
  if not context.interactive then return plain_lines(model.text,style) end
  local rendered=markdown_lines(model.text,style,context.markdown~=false); if label then table.insert(rendered[1].spans,1,span(label.."  ","bold")) end
  return misa.layout.wrap_spans(rendered,context.columns,{{text="▏ ",style="accent"}})
end
local function one(text,style) return {lines={{spans={span(text,style)}}}} end
return {setup=function()
  assert(misa.layout,"component.message requires layout")
  local function reg(role,render) misa.reg_component("default."..role,{render=render}) end
  reg("transcript.user",function(model,context) return {lines=context.interactive and message(model,context,"user","You") or {}} end)
  reg("transcript.assistant",function(model,context) return {lines=message(model,context,"assistant","Assistant")} end)
  reg("transcript.thinking",function(model,context) return {lines=message(model,context,"thinking","Thinking")} end)
  reg("transcript.thinking_collapsed",function(model) return one("▏ Thinking  "..tostring(model.summary or "collapsed"),"thinking") end)
  reg("transcript.tool_call",function(model) return one("▏ Tool  "..tostring(model.name or "tool").."  "..tostring(model.detail or "collapsed"),"tool") end)
  reg("transcript.tool_result",function(model,context) if model.collapsed then return one("▏ "..(model.is_error and "Tool error" or "Tool result").."  collapsed",model.is_error and "error" or "tool") end return {lines=message(model,context,model.is_error and "error" or "tool",model.is_error and "Tool error" or "Tool result")} end)
  reg("transcript.harness",function(model,context) return {lines=message(model,context,model.level=="error" and "error" or "plain",nil)} end)
end}
