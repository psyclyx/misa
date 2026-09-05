-- Shared choice-session policy for inline and overlay presentations. This service
-- owns actions, filtering, views, rows, selection, and picker-visible geometry.
local views, sealed = {}, false
local key_context = "choices"
local default_banks = {
  {"1","2","3","4","5","6","7","8","9"},
  {"q","w","e","r","t","y","u","i","o"},
  {"a","s","d","f","g","h","j","k","l"},
}
local default_actions = {
  enter="accept", escape="cancel", ctrl_c="cancel", ctrl_d="cancel", eof="cancel",
  arrow_up="previous", arrow_down="next", tab="accept", arrow_right="cycle",
  arrow_left="cycle_previous", ["alt+p"]="cycle_previous", ["alt+v"]="favorite",
  ["alt+/"]="replace_view", ["alt+space"]="open_overlay",
}

local function copy_item(item)
  assert(type(item)=="table" and type(item.value)=="string" and item.value~="", "choice value must be nonempty")
  assert(item.label==nil or type(item.label)=="string", "choice label must be a string")
  assert(item.description==nil or type(item.description)=="string", "choice description must be a string")
  assert(item.search==nil or type(item.search)=="string" or type(item.search)=="table", "choice search must be a string or array")
  local search
  if type(item.search)=="table" then
    search={}; for _,value in ipairs(item.search) do assert(type(value)=="string", "choice search fields must be strings"); search[#search+1]=value end
  else search=item.search end
  return {value=item.value,label=item.label,description=item.description,search=search}
end

local function copy_items(source)
  assert(type(source)=="table", "choice items must be an array")
  local result,seen={},{}
  for _,item in ipairs(source) do local copy=copy_item(item); assert(not seen[copy.value], "choice values must be unique"); seen[copy.value]=true; result[#result+1]=copy end
  return result
end

local function searchable(item)
  local fields={item.value,item.label or "",item.description or ""}
  if type(item.search)=="string" then fields[#fields+1]=item.search
  elseif type(item.search)=="table" then for _,value in ipairs(item.search) do fields[#fields+1]=value end end
  return table.concat(fields," ")
end

local function matches(source,query)
  if misa.fuzzy_choices then return misa.fuzzy_choices(source,query,searchable) end
  local result,needle={},query:lower()
  for _,item in ipairs(source) do if needle=="" or searchable(item):lower():find(needle,1,true) then result[#result+1]=item end end
  return result
end

local function preference(db,scope,value)
  local scopes=db.preferences and db.preferences.scopes
  return scopes and scopes[scope] and scopes[scope][value] or nil
end

local function flat_project(session,db,definition)
  local source={}
  for _,item in ipairs(session.items) do if not definition.include or definition.include(item,session,db) then source[#source+1]=item end end
  if definition.order then table.sort(source,function(a,b) return definition.order(a,b,session,db) end) end
  return matches(source,session.query)
end

-- Nodes retain their trailing delimiter. That makes moving to a parent one
-- state transition for both `/command/groups` and provider-qualified models.
local function delimiter_at(value,start)
  local at=value:find("[/.:]",start)
  return at
end
local function parent_node(node)
  if node=="" then return "" end
  local trimmed=node:sub(1,-2); if trimmed=="" then return "" end
  local last
  for at in trimmed:gmatch("()[/.:]") do last=at end
  return last and trimmed:sub(1,last) or ""
end
local function leaf_name(value,node)
  local rest=value:sub(#node+1)
  return rest~="" and rest or value
end
local function tree_project(session)
  local node=session.tree.node
  local candidates=session.query=="" and session.items or matches(session.items,session.query)
  local groups,order={},{}
  for _,item in ipairs(candidates) do if item.value:sub(1,#node)==node then
    local rest=item.value:sub(#node+1); local delimiter=delimiter_at(rest,1)
    -- A leading delimiter is the root typed by slash commands, not an empty group.
    if delimiter==1 then delimiter=delimiter_at(rest,2) end
    if delimiter and delimiter<#rest then
      local prefix=node..rest:sub(1,delimiter)
      local group=groups[prefix]
      if not group then group={value=prefix,label=rest:sub(1,delimiter),search="",_tree_prefix=prefix}; groups[prefix]=group; order[#order+1]=prefix end
      group.search=group.search.." "..searchable(item)
    else
      local projected={value=item.value,label=item.label,description=item.description,search=searchable(item),_source=item}
      if projected.label==nil or projected.label=="" or projected.label==projected.value then projected.label=leaf_name(item.value,node) end
      groups[item.value]=projected; order[#order+1]=item.value
    end
  end end
  local result,seen={},{}
  for _,key in ipairs(order) do if not seen[key] then seen[key]=true; result[#result+1]=groups[key] end end
  return result
end

local function view_compatible(id,session)
  local implementation=views[id]
  return implementation and (not implementation.compatible or implementation.compatible(session))
end
local function configured_views(config,purpose)
  local purposes=type(config)=="table" and config.purposes or nil
  local selected=type(purposes)=="table" and (purposes[purpose] or purposes[purpose:gsub("_","-")]) or nil
  if selected==nil then selected={"all"} end
  assert(type(selected)=="table" and #selected>0, "choice purpose must select at least one view")
  local result={}; for _,id in ipairs(selected) do assert(type(id)=="string" and views[id], "unknown choice view: "..tostring(id)); result[#result+1]=id end
  return result
end

local function project_one(session,slot,db)
  local id,definition=session.view_ids[slot],session.custom_views and session.custom_views[slot]
  local implementation=definition or assert(views[id],"unknown choice view: "..tostring(id))
  local items=definition and matches(definition.items,session.query) or (implementation.project and implementation.project(session,db,implementation) or flat_project(session,db,implementation))
  local state=session.view_state[id] or {highlight=0}; session.view_state[id]=state
  local old=state.items and state.items[state.highlight]
  state.items,state.highlight=items,#items>0 and 1 or 0
  if old then for index,item in ipairs(items) do if item.value==old.value then state.highlight=index; break end end end
  return {id=id,title=implementation.title or id,items=items,filtered=items,highlight=state.highlight}
end
local function refresh(session,db)
  session.panels={}; for slot=1,#session.view_ids do session.panels[slot]=project_one(session,slot,db) end
  return session
end
local function active(session) return session.panels[1],session.view_state[session.view_ids[1]] end
local function pop_utf8(value)
  local index=#value; while index>0 and value:byte(index)>=0x80 and value:byte(index)<0xc0 do index=index-1 end
  return value:sub(1,math.max(0,index-1))
end
local function first_index(panel,room)
  if room<=0 or #panel.items<=room then return 1 end
  return math.max(1,math.min(panel.highlight-math.floor(room/2),#panel.items-room+1))
end
local function action_for(event)
  if misa.keybinding_action then return misa.keybinding_action(key_context,event) end
  local key=event.kind=="key" and event.key or event.kind=="alt" and type(event.text)=="string" and ("alt+"..event.text:lower()) or event.kind
  local action=default_actions[key]; if action then return action end
  for bank,keys in ipairs(default_banks) do for slot,value in ipairs(keys) do if key=="alt+"..value then return "option_"..bank.."_"..slot end end end
end
local function hint_for(action)
  if misa.keybinding_hint then return misa.keybinding_hint(key_context,action) end
  local bank,slot=action:match("^option_(%d+)_(%d+)$"); bank,slot=tonumber(bank),tonumber(slot)
  if bank and default_banks[bank] and default_banks[bank][slot] then return "alt+"..default_banks[bank][slot] end
  for key,value in pairs(default_actions) do if value==action then return key end end
end
local function positional(session,action,visible,room)
  visible=math.min(math.max(0,visible or 0),#session.panels,#default_banks)
  room=math.min(math.max(0,room or 0),9)
  for bank=1,visible do for slot=1,room do if action=="option_"..bank.."_"..slot then
    local panel=session.panels[bank]
    return panel.items[first_index(panel,room)+slot-1]
  end end end
end
local function hotkeys(session,visible,room)
  local result={}; visible=math.min(math.max(0,visible or 0),#session.panels,#default_banks); room=math.min(math.max(0,room or 0),9)
  for bank=1,visible do result[bank]={}; local panel=session.panels[bank]; local first=first_index(panel,room)
    for slot=1,room do if panel.items[first+slot-1] then result[bank][first+slot-1]=hint_for("option_"..bank.."_"..slot) end end
  end
  return result
end

local function new_session(spec,db)
  assert(type(spec)=="table" and type(spec.title)=="string" and spec.title~="", "choice session requires a title")
  local purpose=spec.purpose or "generic"
  local session={title=spec.title,purpose=purpose,items=copy_items(spec.items or {}),query=spec.query or "",selected=spec.selected~=misa.json_null and spec.selected or nil,
    preference_scope=spec.preference_scope,view_state={},tree={node=spec.tree_node or ""}}
  if spec.view_definitions then
    session.view_ids,session.custom_views={},{}
    for index,definition in ipairs(spec.view_definitions) do local id=assert(definition.id); session.view_ids[index],session.custom_views[index]=id,{id=id,title=definition.title,items=copy_items(definition.items or {})} end
  elseif spec.views then session.view_ids={}; for index,id in ipairs(spec.views) do session.view_ids[index]=id end
  else session.view_ids=configured_views(misa.choice_config,purpose) end
  if not session.custom_views then for _,id in ipairs(session.view_ids) do assert(view_compatible(id,session),"incompatible choice view: "..id) end end
  return refresh(session,db)
end
local function row(item,highlighted,selected,hotkey)
  local label=item.label and item.label~="" and item.label or item.value
  local description=item.description
  if description==label or description==item.value then description=nil end
  return {marker=highlighted and ">" or (selected and "✓" or " "),hotkey=hotkey,label=label,value=item.value,description=description,selected=selected,active=highlighted}
end
local function picker_layout(session,db,terminal)
  assert(misa.layout,"picker choice layout requires layout")
  refresh(session,db)
  local height=math.max(0,math.floor(terminal.lines or 0))
  local available=terminal.available_lines
  if available==nil then available=misa.picker_available_lines and misa.picker_available_lines(db,terminal) or height end
  available=math.max(0,math.floor(available))
  local widths=misa.layout.columns(math.max(1,terminal.columns or 1),28,math.min(#session.panels,3),2)
  local result={columns={},targets={},available_lines=available,panel_count=#widths}
  local panel_budget=math.max(0,available-2)
  for bank,width in ipairs(widths) do
    local panel=session.panels[bank]; local title=panel.title..(bank==1 and " *" or "")
    local used=#misa.layout.wrap_spans({{spans={{text=title}}}},width)
    local start=first_index(panel,math.min(9,#panel.items)); local highlight=panel.highlight
    local function item_height(index,slot)
      local item=panel.items[index]; local hotkey=hint_for("option_"..bank.."_"..slot); local model=row(item,index==highlight,item.value==session.selected,hotkey)
      local description=model.description and model.description~="" and " — "..model.description or ""; local hint=hotkey and hotkey.." " or ""
      return #misa.layout.wrap_spans({{spans={{text=model.marker.." "..hint..model.label..description}}}},width)
    end
    while start<highlight and used<panel_budget do
      local total=used; for index=start,highlight do total=total+item_height(index,index-start+1) end
      if total<=panel_budget then break end; start=start+1
    end
    local rows={}; for index=start,math.min(#panel.items,start+8) do
      if used>=panel_budget then break end
      local slot=index-start+1; local hotkey=hint_for("option_"..bank.."_"..slot); local model=row(panel.items[index],index==highlight,panel.items[index].value==session.selected,hotkey)
      rows[#rows+1]=model; result.targets["option_"..bank.."_"..slot]=panel.items[index]
      used=used+item_height(index,slot)
    end
    result.columns[bank]={id=panel.id,title=panel.title,rows=rows}
  end
  return result
end

return {setup=function(context)
  local config=type(context.config)=="table" and context.config.choices or nil; config=type(config)=="table" and config or {}; misa.choice_config=config
  local declarations={accept={"enter","tab"},cancel={"escape","ctrl_c","ctrl_d","eof"},previous={"arrow_up"},next={"arrow_down"},cycle={"arrow_right"},cycle_previous={"arrow_left","alt+p"},favorite={"alt+v"},replace_view={"alt+/"},open_overlay={"alt+space"}}
  for action,keys in pairs(declarations) do misa.reg_keybinding({context=key_context,action=action,default=keys}) end
  for bank,keys in ipairs(default_banks) do for slot,key in ipairs(keys) do misa.reg_keybinding({context=key_context,action="option_"..bank.."_"..slot,default={"alt+"..key}}) end end

  misa.reg_choice_view=function(id,implementation)
    assert(not sealed,"choice view registrations are sealed"); assert(type(id)=="string" and id~="" and views[id]==nil,"invalid or duplicate choice view")
    assert(type(implementation)=="table","choice view must be a table"); views[id]=implementation
  end
  misa.reg_choice_view("all",{title="All"})
  misa.reg_choice_view("favorites",{title="Favorites",include=function(item,session,db) local p=preference(db,session.preference_scope,item.value); return p and p.favorite==true end})
  misa.reg_choice_view("frecency",{title="Recent",include=function(item,session,db) local p=preference(db,session.preference_scope,item.value); return p and (p.uses or 0)>0 end,
    order=function(a,b,session,db) local pa,pb=preference(db,session.preference_scope,a.value),preference(db,session.preference_scope,b.value); return ((pa.last or 0)*1000000+(pa.uses or 0))>((pb.last or 0)*1000000+(pb.uses or 0)) end})
  misa.reg_choice_view("slash-prefix",{title="Tree",compatible=function(session) return session.purpose=="command-completion" or session.purpose=="models" end,project=tree_project})

  misa.choice_session=new_session
  misa.choice_refresh=refresh
  misa.choice_action=action_for
  misa.choice_hint=hint_for
  misa.choice_first_index=first_index
  misa.choice_positional=positional
  misa.choice_hotkeys=hotkeys
  misa.choice_picker_layout=picker_layout
  misa.choice_set_items=function(session,items,db) session.items=copy_items(items); return refresh(session,db) end
  misa.choice_accept=function(session,item,db)
    if item._tree_prefix then session.tree.node=item._tree_prefix; session.query=""; refresh(session,db); return {consumed=true,tree_changed=true,node=session.tree.node} end
    return {consumed=true,accepted=item._source or item}
  end
  misa.choice_registered_views=function(session) local result={}; for id,implementation in pairs(views) do if view_compatible(id,session) then result[#result+1]={value=id,label=implementation.title or id,search=id} end end; table.sort(result,function(a,b)return a.value<b.value end); return result end
  misa.choice_replace_view=function(session,id,db) assert(view_compatible(id,session),"incompatible choice view: "..tostring(id)); session.view_ids[1]=id; if session.custom_views then session.custom_views[1]=false end; return refresh(session,db) end
  misa.choice_rows=function(session,db,row_hotkeys)
    refresh(session,db); local result={}
    for panel_index,panel in ipairs(session.panels) do local rows={}; for index,item in ipairs(panel.items) do rows[index]=row(item,index==panel.highlight,item.value==session.selected,row_hotkeys and row_hotkeys[panel_index] and row_hotkeys[panel_index][index]) end
      result[panel_index]={id=panel.id,title=panel.title,rows=rows} end
    return result
  end
  misa.choice_input=function(session,event,db)
    refresh(session,db); local panel,state=active(session); local action=event.action
    if event.kind=="text" and type(event.text)=="string" then
      session.query=session.query..event.text; refresh(session,db); panel=active(session)
      if session.view_ids[1]=="slash-prefix" and session.query~="" and #panel.items==1 and panel.items[1]._tree_prefix then
        local group,query=panel.items[1],session.query
        session.tree.node=group._tree_prefix
        -- Prefix typing has become the node path; hidden-label search remains a
        -- query inside the entered node so its match does not disappear.
        if group._tree_prefix:lower():find(query:lower(),1,true) then session.query="" end
        refresh(session,db); return {consumed=true,tree_changed=true,node=session.tree.node}
      end
    elseif event.kind=="backspace" then
      if session.view_ids[1]=="slash-prefix" and session.query=="" and session.tree.node~="" then session.tree.node=parent_node(session.tree.node); refresh(session,db); return {consumed=true,tree_changed=true,node=session.tree.node}
      else session.query=pop_utf8(session.query) end
    elseif action=="previous" and #panel.items>0 then state.highlight=state.highlight<=1 and #panel.items or state.highlight-1
    elseif action=="next" and #panel.items>0 then state.highlight=state.highlight>=#panel.items and 1 or state.highlight+1
    elseif action=="cycle" then local first=table.remove(session.view_ids,1); table.insert(session.view_ids,first); if session.custom_views then local custom=table.remove(session.custom_views,1); table.insert(session.custom_views,custom) end
    elseif action=="cycle_previous" then local last=table.remove(session.view_ids); table.insert(session.view_ids,1,last); if session.custom_views then local custom=table.remove(session.custom_views); table.insert(session.custom_views,1,custom) end
    elseif action=="replace_view" then return {consumed=true,replace_view=true}
    elseif action=="open_overlay" then return {consumed=true,open_overlay=true}
    elseif action=="favorite" and session.preference_scope and panel.highlight>0 then return {consumed=true,favorite=panel.items[panel.highlight].value}
    elseif action=="cancel" then return {consumed=true,cancelled=true}
    elseif action=="accept" and panel.highlight>0 then return misa.choice_accept(session,panel.items[panel.highlight],db)
    else return {consumed=false} end
    refresh(session,db); return {consumed=true}
  end

  -- Empty commands with candidates use the same overlay session as every other choice.
  misa.reg_interceptor({id="choices/command",before=function(tx)
    local event=tx.event
    if type(event.command)~="string" or event.resumed_choice or (type(event.arguments)=="string" and event.arguments:match("%S")) then return tx end
    local command=misa.command(event.command); if not command or not (command.completion or command.complete) then return tx end
    tx.event={type="choices/command-open",command=command.name}; return tx
  end})
  misa.reg_event("choices/command-open",function(db,event)
    local command=assert(misa.command(event.command)); db.choice_commands=db.choice_commands or {sequence=0,pending={}}; local state=db.choice_commands
    state.sequence=state.sequence+1; local token="command:"..state.sequence; state.pending[token]={event=command.event,command=command.name}
    return {db=db,fx={{type="dispatch",event={type="picker/open",id="command-choice",token=token,title=command.name:sub(2),completion="choices/command-selected",purpose=command.choice_purpose or "command",items=misa.command_completions(command,"",db),selected=command.selected and command.selected(db) or nil,preference_scope=command.preference_scope or ("command:"..command.name)}}}}
  end)
  misa.reg_event("choices/command-selected",function(db,event)
    local pending=db.choice_commands and db.choice_commands.pending[event.picker_token]; if event.picker~="command-choice" or not pending then return end
    db.choice_commands.pending[event.picker_token]=nil; if event.cancelled then return {db=db,fx={{type="terminal/read"}}} end
    return {db=db,fx={{type="dispatch",event={type=pending.event,command=pending.command,arguments=event.value,resumed_choice=true}}}}
  end)
  misa.reg_interceptor({id="choices/seal",before=function(tx) if tx.event.type=="app/start" then sealed=true end; return tx end})
end}
