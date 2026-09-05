-- Common indicator visual. Semantic label, value, and hotkey classes stay
-- distinct, while low-priority values disappear as the terminal narrows.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  local function render_indicators(model,context)
    local source=model.indicators or {}; local keep={}; for i=1,#source do keep[i]=true end
    local function item_width(item)
      local width=misa.layout.width(tostring(item.label or "").." "..tostring(item.value or ""))
      if item.hotkey and item.hotkey~="" then local rendered=""; for _,token in ipairs(misa.keybinding_tokens and misa.keybinding_tokens(item.hotkey) or {{text=item.hotkey}}) do rendered=rendered..token.text end; width=width+1+misa.layout.width(rendered) end
      return width
    end
    local function total()
      local width,count=0,0; for i,item in ipairs(source) do if keep[i] then width=width+item_width(item); count=count+1 end end
      return width+math.max(0,count-1)*2
    end
    local columns=math.max(1,math.floor(tonumber(context.columns) or 80))
    while total()>columns do
      local victim=nil
      for i,item in ipairs(source) do if keep[i] and (not victim or (tonumber(item.priority) or 0)<(tonumber(source[victim].priority) or 0) or ((tonumber(item.priority) or 0)==(tonumber(source[victim].priority) or 0) and i>victim)) then victim=i end end
      if not victim then break end; keep[victim]=false
    end
    local spans={}
    for i,item in ipairs(source) do if keep[i] then
      if #spans>0 then spans[#spans+1]=span("  ","plain") end
      spans[#spans+1]=span(tostring(item.label or ""),"label")
      spans[#spans+1]=span(" ","plain")
      spans[#spans+1]=span(tostring(item.value or ""),"value")
      if item.hotkey and item.hotkey~="" then spans[#spans+1]=span(" ","plain"); for _,key_span in ipairs(misa.render_keybinding and misa.render_keybinding(item.hotkey) or {span(tostring(item.hotkey),"keybinding")}) do spans[#spans+1]=key_span end end
    end end
    return {lines=#spans>0 and {{spans=spans}} or {}}
  end
  misa.reg_component("default.status.indicators",{render=render_indicators})
  -- Compatibility for custom profiles using the former metrics role.
  misa.reg_component("default.status.metrics",{render=function(model)
    local spans={}; for index,metric in ipairs(model.metrics or {}) do
      if index>1 then spans[#spans+1]=span("  ","plain") end
      spans[#spans+1]=span(tostring(metric.prefix or ""),"dim"); spans[#spans+1]=span(" ","plain"); spans[#spans+1]=span(tostring(metric.value or ""),metric.style or "plain")
    end
    return {lines=#spans>0 and {{spans=spans}} or {}}
  end})
end}
