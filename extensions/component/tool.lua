-- Reusable tool-section visuals. A single section presents the call metadata,
-- arguments, lifecycle indicator, and eventual result.
local function span(text,style) return {text=text,style=style} end
local function composed(base,modifier)
  if not modifier then return base end
  if type(base)~="table" then return {base,modifier} end
  local result={}; for _,token in ipairs(base) do result[#result+1]=token end; result[#result+1]=modifier; return result
end
local function status_style(status)
  if status=="success" then return "tool.success" end
  if status=="error" then return "tool.error" end
  if status=="cancelled" then return "tool.cancelled" end
  return "tool.pending"
end
local function title(model,label)
  local status=model.status or (model.is_error and "error" or "success")
  local parts={span(label,composed(status_style(status),"bold"))}
  if model.description and model.description~="" then parts[#parts+1]=span("  "..model.description,"dim") end
  if model.timestamp then parts[#parts+1]=span("  "..tostring(model.timestamp),"dim") end
  parts[#parts+1]=span("  "..status,status_style(status))
  return {spans=parts}
end
local function bodies(text,style,rail)
  local result={}; text=tostring(text or ""):gsub("\r\n","\n"):gsub("\r","\n")
  for line in (text.."\n"):gmatch("(.-)\n") do result[#result+1]={spans={span("┃ ",rail),span(line,style)}} end
  return result
end
local function append(target,source) for _,line in ipairs(source) do target[#target+1]=line end end
return {setup=function()
  assert(misa.layout,"component.tool requires layout")
  local function render(model,context)
    local fallback=model.kind=="tool_result" and not model.name
    local label=fallback and (model.is_error and "Tool error" or "Tool result") or ("Tool · "..tostring(model.name or "tool"))
    local style=status_style(model.status or (model.is_error and "error" or "success")); local rail=model.rail or (model.is_error and "rail.error" or "rail.tool")
    local lines={title(model,label)}
    if not fallback then append(lines,bodies("args  "..tostring(model.detail or "summary"),style,rail)) end
    if model.result~=nil then append(lines,bodies("result  "..tostring(model.result_detail or model.result),style,rail))
    elseif fallback then append(lines,bodies(model.collapsed and "summary" or model.text,style,rail)) end
    return {lines=misa.layout.wrap_spans(lines,context.columns or 80)}
  end
  misa.reg_component("default.transcript.tool_call",{render=render})
  misa.reg_component("default.transcript.tool_result",{render=render})
end}
