-- Overlay adapter for shared choice sessions. Geometry is supplied to the
-- shared positional resolver; picker-specific code owns only modal lifecycle.
local function definitions(event)
  if event.panels==nil then return nil end
  assert(type(event.panels)=="table" and #event.panels>0,"picker panels must be a nonempty array")
  local result={}
  for index,panel in ipairs(event.panels) do result[index]={id=panel.id or tostring(index),title=panel.title or panel.id or tostring(index),items=panel.items or {}} end
  return result
end
local function finish(state,item,cancelled)
  local event={type=state.completion,picker=state.id,picker_token=state.token,value=item and item.value or misa.json_null,cancelled=cancelled==true}
  local fx={{type="dispatch",event=event}}
  if item and state.session.preference_scope then fx[#fx+1]={type="dispatch",event={type="choice/used",scope=state.session.preference_scope,value=item.value}} end
  return fx
end
local function open_state(event,db,parent)
  assert(type(event.id)=="string" and event.id~="" and type(event.token)=="string" and event.token~="","invalid picker identity")
  assert(type(event.completion)=="string" and event.completion~="","invalid picker completion")
  local session=event.session
  if session then
    assert(type(session)=="table" and type(session.title)=="string" and type(session.panels)=="table","invalid choice session")
    misa.choice_refresh(session,db)
  else
    local defs=definitions(event); local items=event.items or {}
    if defs then items={}; local seen={}; for _,definition in ipairs(defs) do for _,item in ipairs(definition.items) do if not seen[item.value] then seen[item.value]=true; items[#items+1]=item end end end end
    session=misa.choice_session({title=event.title,purpose=event.purpose or "generic",items=items,selected=event.selected,preference_scope=event.preference_scope,views=event.views,view_definitions=defs},db)
  end
  return {id=event.id,token=event.token,completion=event.completion,session=session,parent=parent}
end
local function view_picker(parent,db)
  return open_state({id="picker-picker",token=parent.token..":views",title="Choose view",completion="picker/replace-view",items=misa.choice_registered_views(parent.session),views={"all"}},db,parent)
end
return {setup=function()
  assert(misa.choice_session and misa.choice_action and misa.choice_picker_layout,"picker requires choices")
  misa.picker=true
  misa.reg_event("picker/open",function(db,event)
    assert(not db.picker,"a picker is already open")
    local state=open_state(event,db,nil); db.picker=event.choose_view and view_picker(state,db) or state
    return {db=db,fx={{type="terminal/read"}}}
  end)
  misa.reg_event("picker/update",function(db,event)
    local state=db.picker; if not state or event.id~=state.id or event.token~=state.token then return end
    if event.items then misa.choice_set_items(state.session,event.items,db)
    elseif event.panels then
      local replacement=open_state({id=state.id,token=state.token,completion=state.completion,title=state.session.title,purpose=state.session.purpose,panels=event.panels,selected=event.selected~=nil and event.selected or state.session.selected,preference_scope=state.session.preference_scope},db,state.parent)
      db.picker=replacement; state=replacement
    end
    if event.selected~=nil then state.session.selected=event.selected~=misa.json_null and event.selected or nil; misa.choice_refresh(state.session,db) end
    return {db=db}
  end)
  misa.reg_interceptor({id="picker/input",before=function(tx)
    if tx.event.type=="terminal/input" and tx.db.picker then tx.event={type="picker/input",kind=tx.event.kind,text=tx.event.text,key=tx.event.key} end
    return tx
  end})
  misa.reg_event("picker/input",function(db,event,cofx)
    local state=assert(db.picker); local action=misa.choice_action(event)
    local geometry=misa.choice_picker_layout(state.session,db,cofx.terminal)
    local item=geometry.targets[action]
    local result=item and misa.choice_accept(state.session,item,db) or misa.choice_input(state.session,{kind=event.kind,text=event.text,action=action},db)
    if result.replace_view then
      db.picker=view_picker(state,db)
    elseif result.accepted and state.id=="picker-picker" and state.parent then
      local parent=state.parent; misa.choice_replace_view(parent.session,result.accepted.value,db); db.picker=parent
    elseif result.cancelled and state.id=="picker-picker" and state.parent then
      db.picker=state.parent
    elseif result.accepted then
      db.picker=nil; return {db=db,fx=finish(state,result.accepted,false)}
    elseif result.cancelled then
      db.picker=nil; return {db=db,fx=finish(state,nil,true)}
    elseif result.favorite then
      return {db=db,fx={{type="dispatch",event={type="preferences/toggle",scope=state.session.preference_scope,value=result.favorite}},{type="terminal/read"}}}
    end
    return {db=db,fx={{type="terminal/read"}}}
  end)
end}
