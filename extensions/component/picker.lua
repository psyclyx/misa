-- Pure renderer for the complete geometry produced by choice_layout.
local function span(text,style) return {text=text,style=style} end
local function key_spans(key) return misa.render_keybinding and misa.render_keybinding(key) or {span(tostring(key or ""),"keybinding")} end
local function append(target,source) for _,value in ipairs(source or {}) do target[#target+1]=value end end
local function fit_line(line,width)
  local spans,remaining={},width
  for _,part in ipairs(line and line.spans or {}) do if remaining>0 then local text,_,used=misa.layout.take(part.text or "",remaining); spans[#spans+1]={text=text,style=part.style,link=part.link}; remaining=remaining-used end end
  if remaining>0 then spans[#spans+1]=span(string.rep(" ",remaining),"plain") end; return spans
end
return {setup=function()
  assert(misa.layout,"component.picker requires layout")
  misa.reg_component("default.picker",{render=function(model)
    local pad=string.rep(" ",model.x or 0); local result={}
    result[#result+1]={spans={span(pad,"plain"),span(model.input.title.."> ","choice.prompt"),span(model.input.text,"choice.query")}}
    for index=1,model.preview.height do result[#result+1]={spans={span(pad,"plain"),span(model.preview.lines[index] or "","choice.preview")}} end
    local rendered={}
    for bank,column in ipairs(model.columns) do
      local lines={{spans={span(column.title,column.active and "choice.view.active" or "choice.view")}}}
      if #column.rows==0 then lines[#lines+1]={spans={span("  no matches","choice.empty")}} end
      for _,row in ipairs(column.rows) do
        local style=row.selected and "choice.row.selected" or (row.active and "choice.row.active" or "choice.row")
        local spans={span((row.marker or " ").." ",style)}
        if row.hotkey then append(spans,key_spans(row.hotkey)); spans[#spans+1]=span(" ","plain") end
        spans[#spans+1]=span(row.label,style); if row.description and row.description~="" then spans[#spans+1]=span(" — "..row.description,"choice.hint") end
        local wrapped=misa.layout.wrap_spans({{spans=spans}},column.width); append(lines,wrapped)
      end
      rendered[bank]=lines
    end
    for line_index=1,model.panel_height do
      local spans={span(pad,"plain")}; local any=false
      for bank,column in ipairs(model.columns) do local line=rendered[bank][line_index]; if line then any=true end
        append(spans,fit_line(line,column.width)); if bank<#model.columns then spans[#spans+1]=span("  ","plain") end
      end
      if not any then break end; result[#result+1]={spans=spans}
    end
    local hint_spans={span(pad,"plain")}; append(hint_spans,misa.render_keybinding_reference and misa.render_keybinding_reference(model.hints) or {})
    result[#result+1]={spans=hint_spans}
    while #result>model.height do table.remove(result) end
    return {lines=result,cursor={row=1,byte=#pad+#model.input.title+2+model.input.cursor},exclusive=false,overlay=true,height=model.height}
  end})
end}
