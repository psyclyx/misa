-- Single geometry projection for every choice overlay. Input handling and the
-- component consume the same rows and positional targets from this projection.
local function clamp(value,low,high) return math.max(low,math.min(high,value)) end
local function preview_lines(preview)
  if preview==nil then return {} end
  if type(preview)=="string" then return {preview} end
  local result={}
  if type(preview)=="table" then
    if preview.title then result[#result+1]=tostring(preview.title) end
    if type(preview.lines)=="table" then for _,line in ipairs(preview.lines) do result[#result+1]=tostring(line) end
    else local body={}; for key,value in pairs(preview) do if key~="title" and key~="lines" then body[#body+1]=tostring(key)..": "..tostring(value) end end; table.sort(body); for _,line in ipairs(body) do result[#result+1]=line end end
  end
  return result
end
return {setup=function(context)
  assert(misa.layout and misa.choice_rows,"choice_layout requires layout and choices")
  local configured=type(context.config)=="table" and type(context.config.choices)=="table" and context.config.choices.overlay or nil; configured=type(configured)=="table" and configured or {}
  for _,name in ipairs({"preferred_width","min_width","max_width","preferred_height","min_height","max_height","panel_min_width"}) do local value=configured[name]; assert(value==nil or (type(value)=="number" and value>=1 and value%1==0),"choice overlay "..name.." must be a positive integer") end
  assert((configured.min_width or 28)<=(configured.max_width or 100),"choice overlay min_width exceeds max_width")
  assert((configured.min_height or 4)<=(configured.max_height or 18),"choice overlay min_height exceeds max_height")
  misa.choice_picker_layout=function(session,db,terminal)
    misa.choice_refresh(session,db)
    local screen_width=math.max(1,math.floor(terminal.columns or 1)); local available=terminal.available_lines
    if available==nil then available=misa.picker_available_lines and misa.picker_available_lines(db,terminal) or terminal.lines or 0 end
    available=math.max(0,math.floor(available))
    local min_width=math.min(screen_width,configured.min_width or 28); local max_width=math.min(screen_width,configured.max_width or 100)
    local width=clamp(configured.preferred_width or 84,min_width,max_width); local x=math.floor((screen_width-width)/2)
    local min_height=math.min(available,configured.min_height or 4); local max_height=math.min(available,configured.max_height or 18)
    local height=clamp(configured.preferred_height or 14,min_height,max_height)
    local widths=misa.layout.columns(width,configured.panel_min_width or 28,math.min(#session.panels,3),2)
    local highlighted=session.panels[1] and session.panels[1].items[session.panels[1].highlight]
    local preview=preview_lines(highlighted and highlighted.preview); local preview_height=math.min(#preview,math.max(0,math.floor(height/3)))
    local hint_actions={{"previous","previous"},{"next","next"},{"accept","accept"},{"cancel","cancel"},{"cycle","cycle views"},{"replace_view","views"}}
    if session.preference_scope then hint_actions[#hint_actions+1]={"favorite","favorite"} end
    local hints={}; for _,entry in ipairs(hint_actions) do local key=misa.choice_hint(entry[1]); if key then hints[#hints+1]={key=key,label=entry[2],tokens=misa.keybinding_tokens and misa.keybinding_tokens(key) or nil} end end
    local fixed=1+preview_height+1; local panel_budget=math.max(0,height-fixed)
    local hotkeys=misa.choice_hotkeys(session,#widths,9); local all=misa.choice_rows(session,db,hotkeys); local columns,targets={},{}
    local column_x=0
    for bank,panel_width in ipairs(widths) do
      local panel=session.panels[bank]; local start=misa.choice_first_index(panel,9); local rows,used={},1
      for index=start,math.min(#panel.items,start+8) do
        local model=all[bank].rows[index]; local hotkey=model.hotkey or ""; if misa.keybinding_text and hotkey~="" then hotkey=misa.keybinding_text(hotkey) end
        local text=(model.marker or " ").." "..hotkey..model.label..(model.description and (" — "..model.description) or "")
        local row_height=#misa.layout.wrap_spans({{spans={{text=text}}}},panel_width)
        -- `used` includes the panel title. A row and its positional target are
        -- admitted together only when every wrapped physical line fits.
        if used+row_height>panel_budget then break end
        rows[#rows+1]=model; used=used+row_height; targets["option_"..bank.."_"..#rows]=panel.items[index]
        if used>=panel_budget then break end
      end
      columns[bank]={id=panel.id,title=panel.title,width=panel_width,x=column_x,y=1+preview_height,rows=rows,active=bank==1}; column_x=column_x+panel_width+2
    end
    local breadcrumb=session.tree and session.tree.node or ""; local query=(session.input_prefix or "")..breadcrumb..session.query
    return {x=x,y=0,width=width,height=height,available_lines=available,panel_count=#columns,input={x=x,y=0,width=width,title=session.title,text=query,cursor=#query},preview={x=x,y=1,width=width,lines=preview,height=preview_height},columns=columns,panel_y=1+preview_height,panel_height=panel_budget,hints=hints,hint_y=1+preview_height+panel_budget,hint_height=1,targets=targets}
  end
end}
