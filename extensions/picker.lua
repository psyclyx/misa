-- Generic searchable, multi-panel, single-selection state and input.
local function pop_utf8(value)
  local index = #value
  while index > 0 and value:byte(index) >= 0x80 and value:byte(index) < 0xc0 do index = index - 1 end
  return value:sub(1, math.max(0, index - 1))
end
local function copy_items(values)
  assert(type(values) == "table", "picker items must be an array")
  local items, seen = {}, {}
  for _, item in ipairs(values) do
    assert(type(item) == "table" and type(item.value) == "string" and item.value ~= "", "picker item value must be nonempty")
    assert(item.label == nil or type(item.label) == "string", "picker item label must be a string")
    assert(item.description == nil or type(item.description) == "string", "picker item description must be a string")
    assert(not seen[item.value], "picker item values must be unique within a panel")
    seen[item.value] = true
    items[#items + 1] = { value = item.value, label = item.label, description = item.description }
  end
  return items
end
local function copy_panels(event)
  local source = event.panels or { { id = "choices", title = event.title, items = event.items or {} } }
  assert(type(source) == "table" and #source > 0, "picker panels must be a nonempty array")
  local result, seen = {}, {}
  for index, panel in ipairs(source) do
    local id = panel.id or tostring(index)
    assert(type(panel) == "table" and type(id) == "string" and id ~= "" and not seen[id], "invalid picker panel")
    seen[id] = true
    result[#result + 1] = { id = id, title = panel.title or id, items = copy_items(panel.items or {}), filtered = {}, highlight = 0 }
  end
  return result
end
local function searchable(item) return item.value .. " " .. (item.label or "") .. " " .. (item.description or "") end
local function filter_panel(panel, query)
  local highlighted = panel.filtered[panel.highlight]
  if misa.fuzzy_choices then panel.filtered = misa.fuzzy_choices(panel.items, query, searchable) else
    panel.filtered = {}; local needle = query:lower()
    for _, item in ipairs(panel.items) do if needle == "" or searchable(item):lower():find(needle, 1, true) then panel.filtered[#panel.filtered + 1] = item end end
  end
  panel.highlight = #panel.filtered > 0 and 1 or 0
  if highlighted then for i, item in ipairs(panel.filtered) do if item.value == highlighted.value then panel.highlight = i; break end end end
end
local function filter(state) for _, panel in ipairs(state.panels) do filter_panel(panel, state.query) end end
local function action_for(event)
  if misa.keybinding_action then return misa.keybinding_action("picker", event) end
  return ({ enter="accept", escape="cancel", ctrl_c="cancel", ctrl_d="cancel", eof="cancel", arrow_up="previous", arrow_down="next", tab="next_panel", arrow_right="next_panel", arrow_left="previous_panel" })[event.kind]
end
local banks = { {"1","2","3","4","5","6","7","8","9"}, {"q","w","e","r","t","y","u","i","o"}, {"a","s","d","f","g","h","j","k","l"} }
local function panel_at(state, slot) return (state.active + slot - 2) % #state.panels + 1 end
local function option_hint(bank, slot)
  if misa.keybinding_hint then return misa.keybinding_hint("picker", "option_"..bank.."_"..slot) end
  return banks[bank] and banks[bank][slot] and ("alt+"..banks[bank][slot]) or nil
end
local function panel_items(panels)
  local result, seen = {}, {}
  for _, panel in ipairs(panels) do for _, item in ipairs(panel.items) do if not seen[item.value] then
    seen[item.value] = true; result[#result + 1] = { value=item.value, label=item.label, description=item.description }
  end end end
  return result
end
local function first_index(panel, room)
  if #panel.filtered <= room then return 1 end
  return math.max(1, math.min(panel.highlight - math.floor(room / 2), #panel.filtered - room + 1))
end
local function complete(state, item)
  local fx = { { type="dispatch", event={ type=state.completion, picker=state.id, picker_token=state.token, value=item.value, cancelled=false } } }
  if state.preference_scope then fx[#fx + 1] = { type="dispatch", event={ type="choice/used", scope=state.preference_scope, value=item.value } } end
  return fx
end
return { setup = function()
  misa.picker = true
  local declarations = { accept={"enter"}, cancel={"escape","ctrl_c","ctrl_d","eof"}, previous={"arrow_up"}, next={"arrow_down"}, next_panel={"tab","arrow_right"}, previous_panel={"arrow_left","alt+p"}, favorite={"alt+v"} }
  for action, keys in pairs(declarations) do misa.reg_keybinding({ context="picker", action=action, default=keys }) end
  for bank, keys in ipairs(banks) do for slot, key in ipairs(keys) do misa.reg_keybinding({ context="picker", action="option_"..bank.."_"..slot, default={"alt+"..key} }) end end
  misa.reg_event("picker/open", function(db, event)
    assert(not db.picker and type(event.id)=="string" and event.id~="" and type(event.token)=="string" and event.token~="", "invalid picker identity")
    assert(type(event.title)=="string" and event.title~="" and type(event.completion)=="string" and event.completion~="", "invalid picker contract")
    local panels=copy_panels(event)
    db.picker={ id=event.id, token=event.token, title=event.title, panels=panels, active=1, source_items=event.items and copy_items(event.items) or panel_items(panels), query="", selected=event.selected~=misa.json_null and event.selected or nil, completion=event.completion, preference_scope=event.preference_scope }
    filter(db.picker); return {db=db,fx={{type="terminal/read"}}}
  end)
  misa.reg_event("picker/update", function(db,event)
    local state=db.picker; if not state or event.id~=state.id or event.token~=state.token then return end
    local active_id=state.panels[state.active].id
    if event.panels~=nil or event.items~=nil then
      state.panels=copy_panels({panels=event.panels,items=event.items,title=state.title}); state.active=1
      for i,panel in ipairs(state.panels) do if panel.id==active_id then state.active=i; break end end
      state.source_items=event.items and copy_items(event.items) or panel_items(state.panels)
    end
    if event.selected==misa.json_null then state.selected=nil elseif event.selected~=nil then state.selected=event.selected end
    filter(state); return {db=db}
  end)
  misa.reg_interceptor({id="picker/input",before=function(tx)
    if tx.event.type=="terminal/input" and tx.db.picker then tx.event={type="picker/input",kind=tx.event.kind,text=tx.event.text,key=tx.event.key} end
    return tx
  end})
  misa.reg_event("picker/input",function(db,event,cofx)
    local state=assert(db.picker,"picker is not open"); local panel,action=state.panels[state.active],action_for(event)
    if event.kind=="text" and type(event.text)=="string" then state.query=state.query..event.text; filter(state)
    elseif event.kind=="backspace" then state.query=pop_utf8(state.query); filter(state)
    elseif action=="previous" and #panel.filtered>0 then panel.highlight=panel.highlight<=1 and #panel.filtered or panel.highlight-1
    elseif action=="next" and #panel.filtered>0 then panel.highlight=panel.highlight>=#panel.filtered and 1 or panel.highlight+1
    elseif action=="next_panel" then state.active=state.active%#state.panels+1
    elseif action=="previous_panel" then state.active=(state.active-2)%#state.panels+1
    elseif action=="favorite" and state.preference_scope and panel.highlight>0 then
      local item=panel.filtered[panel.highlight]; return {db=db,fx={{type="dispatch",event={type="preferences/toggle",scope=state.preference_scope,value=item.value,items=state.source_items,selected=state.selected or misa.json_null,picker=state.id,picker_token=state.token}}}}
    elseif action=="accept" and panel.highlight>0 then local item=panel.filtered[panel.highlight]; db.picker=nil; return {db=db,fx=complete(state,item)}
    elseif action=="cancel" then db.picker=nil; return {db=db,fx={{type="dispatch",event={type=state.completion,picker=state.id,picker_token=state.token,value=misa.json_null,cancelled=true}}}}
    else
      local visible=math.min(#state.panels,3,math.max(1,math.floor(cofx.terminal.columns/28)))
      local room=math.max(0,math.min(9,cofx.terminal.lines-5))
      for bank=1,visible do for slot=1,room do if action=="option_"..bank.."_"..slot and option_hint(bank,slot) then
        local target=state.panels[panel_at(state,bank)]; local item=target.filtered[first_index(target,room)+slot-1]
        if item then db.picker=nil; return {db=db,fx=complete(state,item)} end
      end end end
    end
    return {db=db,fx={{type="terminal/read"}}}
  end)
end }
