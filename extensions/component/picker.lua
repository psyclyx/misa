-- Default overlay visual for semantic choice rows.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  assert(misa.layout,"component.picker requires layout")
  misa.reg_component("default.picker",{render=function(model,context)
    local title,query,columns=model.title,model.query,model.columns
    local hints=(model.cycle_hint or "right").." cycle"..(model.replace_hint and ("    "..model.replace_hint.." replace view") or "")..(model.favorite_hint and ("    "..model.favorite_hint.." favorite") or "")
    local result={{spans={span(title.."> ","choice.prompt"),span(query,"choice.query")}},{spans={span("  "..hints,"choice.hint")}}}
    local widths=misa.layout.columns(context.columns,28,#columns,2); local rendered_columns={}; local room=math.max(0,context.available_lines-2)
    for visible,width in ipairs(widths) do local column=columns[visible]; local cells={{text=column.title..(visible==1 and " *" or ""),style=visible==1 and "choice.view.active" or "choice.view"}}
      for _,row in ipairs(column.rows) do
        local description=row.description and row.description~="" and " — "..row.description or ""; local hotkey=row.hotkey and row.hotkey.." " or ""
        cells[#cells+1]={text=(row.marker or " ").." "..hotkey..row.label..description,style=row.active and "choice.row.active" or (row.selected and "choice.row.selected" or "choice.row")}
      end
      if #column.rows==0 then cells[#cells+1]={text="  no matches",style="choice.empty"} end
      local lines={}; for _,cell in ipairs(cells) do for _,line in ipairs(misa.layout.wrap_spans({{spans={span(cell.text,cell.style)}}},width)) do lines[#lines+1]=line end end; rendered_columns[visible]=lines
    end
    for row_index=1,room do local spans,any={},false
      for visible,width in ipairs(widths) do local line=rendered_columns[visible][row_index]; if line then any=true end; local text,style="","choice.row"
        if line then style=line.spans[1] and line.spans[1].style or style; for _,part in ipairs(line.spans) do text=text..part.text end end
        spans[#spans+1]=span(misa.layout.fit(text,width),style); if visible<#widths then spans[#spans+1]=span("  ","choice.row") end
      end
      if not any then break end; result[#result+1]={spans=spans}
    end
    return {lines=result,cursor={row=1,byte=#title+2+#query},exclusive=true}
  end})
end}
