-- Default generic dialog visual. It knows only the interaction model.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  assert(misa.layout,"component.dialog requires layout")
  misa.reg_component("default.dialog",{render=function(model,context)
    local width=math.max(12,math.min(context.columns,72)); local lines={}
    lines[#lines+1]={spans={span(model.title~="" and model.title or "Interaction","dialog.title")}}
    for _,line in ipairs(misa.layout.wrap_spans({{spans={span(model.message or "","dialog.message")}}},width)) do lines[#lines+1]=line end
    if model.url then
      lines[#lines+1]={spans={span("URL  ","dialog.label"),span(model.url,"dialog.value")}}
    end
    if model.code then lines[#lines+1]={spans={span("Code ","dialog.label"),span(model.code,"dialog.code")}} end
    if model.progress then lines[#lines+1]={spans={span(tostring(model.progress),"dialog.progress")}} end
    local cursor,input_line
    if model.input_enabled then
      input_line={spans={span("> ","dialog.label"),span(model.input or "","dialog.input")}}
      lines[#lines+1]=input_line
    end
    local hints={}
    for _,hint in ipairs(model.hints or {}) do hints[#hints+1]=tostring(hint) end
    for _,action in ipairs(model.actions or {}) do hints[#hints+1]=action.label end
    if model.cancellable then hints[#hints+1]="esc cancel" end
    if #hints>0 then lines[#lines+1]={spans={span(table.concat(hints,"    "),"dialog.hint")}} end
    while #lines>context.available_lines do table.remove(lines,math.min(#lines,math.max(2,#lines-1))) end
    if input_line then for row,line in ipairs(lines) do if line==input_line then cursor={row=row,byte=2+#(model.input or "")} end end end
    return {lines=lines,cursor=cursor,exclusive=true}
  end})
end}
