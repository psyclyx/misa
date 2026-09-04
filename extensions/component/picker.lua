-- Default picker visual. Selection and input policy remain in picker.lua.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  assert(misa.layout,"component.picker requires layout")
  misa.reg_component("default.picker",{render=function(model,context)
    local state,room=model.state,math.max(0,context.available_lines-2)
    local result={{spans={span(state.title.."> ","accent"),span(state.query,"plain")}},
      {spans={span("  "..model.panel_hint.." cycle panels"..model.favorite_hint,"dim")}}}
    local widths=misa.layout.columns(context.columns,28,math.min(model.visible_panels,#state.panels),2)
    local columns={}
    for visible=1,#widths do
      local panel=state.panels[(state.active+visible-2)%#state.panels+1]; local cells={{text=panel.title..(visible==1 and " *" or ""),style=visible==1 and "accent" or "bold"}}
      local first=model.first_index(panel,model.choice_room)
      for index=first,math.min(#panel.filtered,first+model.choice_room-1) do
        local item,hint=panel.filtered[index],model.option_hints[visible] and model.option_hints[visible][index-first+1]
        local marker=(index==panel.highlight and ">" or " ")..(item.value==state.selected and "✓" or " ")
        local description=item.description and item.description~="" and (" — "..item.description) or ""
        cells[#cells+1]={text=marker.." "..(hint and (hint.." ") or "")..(item.label or item.value)..description,style=index==panel.highlight and "accent" or (item.value==state.selected and "bold" or "plain")}
      end
      if #panel.filtered==0 then cells[#cells+1]={text="  no matches",style="dim"} end
      local rendered={}; for _,cell in ipairs(cells) do
        local wrapped=misa.layout.wrap_spans({{spans={span(cell.text,cell.style)}}},widths[visible])
        for _,line in ipairs(wrapped) do rendered[#rendered+1]=line end
      end; columns[visible]=rendered
    end
    for row=1,room do local spans,any={},false
      for visible,width in ipairs(widths) do local line=columns[visible][row]; if line then any=true end
        local text=""; local style="plain"; if line then style=line.spans[1] and line.spans[1].style or style; for _,part in ipairs(line.spans) do text=text..part.text end end
        spans[#spans+1]=span(misa.layout.fit(text,width),style); if visible<#widths then spans[#spans+1]=span("  ","plain") end
      end
      if not any then break end; result[#result+1]={spans=spans}
    end
    return {lines=result,cursor={row=1,byte=#state.title+2+#state.query},exclusive=true}
  end})
end}
