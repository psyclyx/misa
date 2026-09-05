-- Default transcript chrome. Markdown parsing and terminal flow are delegated to
-- the reusable markdown and component.markdown services.
local function span(text,style,link) return {text=text,style=style,link=link} end
local function composed(base,modifier)
  if not modifier then return base end
  if type(base)~="table" then return {base,modifier} end
  local result={}; for _,token in ipairs(base) do result[#result+1]=token end; result[#result+1]=modifier; return result
end
local function title(model,label,style)
  local parts={span(label,composed(style or "plain","bold"))}
  if model.timestamp then parts[#parts+1]=span("  "..tostring(model.timestamp),"dim") end
  if model.streaming then parts[#parts+1]=span("  pending","pending") end
  if model.interrupted then parts[#parts+1]=span("  interrupted","error") end
  if type(model.tokens_per_second)=="number" then parts[#parts+1]=span("  "..string.format("%.1f",model.tokens_per_second).." tok/s","dim") end
  return {spans=parts}
end
local function rail(model) return assert(model.rail,"message model requires a semantic rail token") end
local function body_lines(model,context,style)
  if context.markdown==false then return misa.markdown_view.plain(model.text,style) end
  return misa.markdown_view.render(misa.markdown.parse(model.text),{base=style,columns=math.max(4,(tonumber(context.columns) or 80)-2)})
end
local function message(model,context,style,label)
  if not context.interactive then return misa.markdown_view.plain(model.text,style) end
  local rendered=body_lines(model,context,style)
  rendered=misa.layout.wrap_spans(rendered,context.columns,{{text="┃ ",style=rail(model)}})
  if label then table.insert(rendered,1,title(model,label,style)) end
  return rendered
end
local function titled(model,label,text,style)
  return {lines={title(model,label,style),{spans={span("┃ ",rail(model)),span(text,style)}}}}
end
return {setup=function()
  assert(misa.layout,"component.message requires layout")
  assert(misa.markdown_view and misa.markdown,"component.message requires markdown and component.markdown")
  local function reg(role,render) misa.reg_component("default."..role,{render=render}) end
  reg("transcript.user",function(model,context) return {lines=context.interactive and message(model,context,"user","You") or {}} end)
  reg("transcript.assistant",function(model,context) return {lines=message(model,context,"assistant","Assistant")} end)
  reg("transcript.thinking",function(model,context) return {lines=message(model,context,"thinking","Thinking")} end)
  reg("transcript.thinking_collapsed",function(model) return titled(model,"Thinking",tostring(model.summary or "summary"),"thinking") end)
  reg("transcript.harness",function(model,context) return {lines=message(model,context,model.level=="error" and "error" or "plain",nil)} end)
end}
