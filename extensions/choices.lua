-- Generic choice state and transitions shared by inline and overlay presentations.
local views,sources,sealed={}, {},false
local key_context="choices"
local banks={{"1","2","3","4","5","6","7","8","9"},{"q","w","e","r","t","y","u","i","o"},{"a","s","d","f","g","h","j","k","l"}}
local fallback={enter="accept",tab="accept",escape="cancel",ctrl_c="cancel",ctrl_d="cancel",eof="cancel",arrow_up="previous",arrow_down="next",arrow_right="cycle",arrow_left="cycle_previous",["alt+p"]="cycle_previous",["alt+v"]="favorite",["alt+/"]="replace_view",["alt+space"]="open_overlay"}
local function scalar(v) return type(v)=="string" or type(v)=="number" or type(v)=="boolean" end
local function clone(value)
  if type(value)~="table" then return value end
  local result={}; for key,item in pairs(value) do result[key]=clone(item) end; return result
end
local function item(source)
  assert(type(source)=="table","choice item must be a table")
  local value=source.value~=nil and source.value or source.id
  assert(scalar(value),"choice value must be scalar")
  local id=source.id or (type(value)=="string" and value or tostring(value)); assert(type(id)=="string" and id~="","choice id must be nonempty")
  local display=source.display; local label,description
  if type(display)=="string" then label=display elseif type(display)=="table" then label,description=display.label,display.description end
  label,description=label or source.label or id,description or source.description
  assert(type(label)=="string" and (description==nil or type(description)=="string"),"invalid choice display")
  assert(source.search==nil or type(source.search)=="string" or type(source.search)=="table","choice search must be a string or array")
  local path=source.path or source.tree_path or id
  if type(path)=="table" then path=table.concat(path,"/") end; assert(type(path)=="string","choice path must be a string or array")
  return {id=id,value=value,label=label,description=description,search=clone(source.search),preview=clone(source.preview),path=path,narrow=clone(source.narrow),invocation=source.invocation}
end
local function items(source)
  assert(type(source)=="table","choice items must be an array"); local result,seen={},{}
  for _,raw in ipairs(source) do local value=item(raw); assert(not seen[value.id],"choice ids must be unique"); seen[value.id]=true; result[#result+1]=value end
  return result
end
local function search_text(value)
  local fields,seen={},{}; local function add(field) if type(field)=="string" and field~="" and not seen[field] then seen[field]=true; fields[#fields+1]=field end end
  add(value.id); add(value.path); add(value.label); add(value.description)
  if type(value.search)=="string" then add(value.search) elseif type(value.search)=="table" then for _,field in ipairs(value.search) do add(field) end end
  return table.concat(fields," ")
end
local function matches(source,query)
  if misa.fuzzy_choices then return misa.fuzzy_choices(source,query,search_text) end
  local result,needle={},query:lower(); for _,value in ipairs(source) do if needle=="" or search_text(value):lower():find(needle,1,true) then result[#result+1]=value end end; return result
end
local function preference(db,scope,id) local scopes=db.preferences and db.preferences.scopes; return scopes and scopes[scope] and scopes[scope][id] end
local function compatible(id,session) local view=views[id]; return view and (not view.compatible or view.compatible(session)) end
local function configured(purpose)
  local purposes=type(misa.choice_config)=="table" and misa.choice_config.purposes; local selected=type(purposes)=="table" and (purposes[purpose] or purposes[purpose:gsub("_","-")]) or nil
  selected=selected or {"all"}; assert(type(selected)=="table" and #selected>0,"choice purpose must select at least one view")
  local result={}; for _,id in ipairs(selected) do assert(views[id],"unknown choice view: "..tostring(id)); result[#result+1]=id end; return result
end
local function project(session,slot,db)
  local id=session.view_ids[slot]; local definition=session.custom_views and session.custom_views[slot]; local view=definition or assert(views[id])
  local source={}
  if definition then source=matches(definition.items,session.query)
  elseif view.project then source=view.project(session,matches)
  else for _,value in ipairs(session.items) do if not view.include or view.include(value,session,db) then source[#source+1]=value end end; if view.order then table.sort(source,function(a,b)return view.order(a,b,session,db)end)end; source=matches(source,session.query) end
  local state=session.view_state[id] or {highlight=0}; session.view_state[id]=state; local previous=state.items and state.items[state.highlight]
  state.items,state.highlight=source,#source>0 and 1 or 0
  if previous then for index,value in ipairs(source) do if value.id==previous.id then state.highlight=index; break end end end
  return {id=id,title=view.title or id,items=source,filtered=source,highlight=state.highlight}
end
local function refresh(session,db) session.panels={}; for slot=1,#session.view_ids do session.panels[slot]=project(session,slot,db) end; return session end
local function pop_utf8(value) local at=#value; while at>0 and value:byte(at)>=0x80 and value:byte(at)<0xc0 do at=at-1 end; return value:sub(1,math.max(0,at-1)) end
local function action(event)
  if misa.keybinding_action then return misa.keybinding_action(key_context,event) end
  local key=event.kind=="key" and event.key or event.kind=="alt" and ("alt+"..event.text:lower()) or event.kind; if fallback[key] then return fallback[key] end
  for bank,keys in ipairs(banks) do for slot,keyname in ipairs(keys) do if key=="alt+"..keyname then return "option_"..bank.."_"..slot end end end
end
local function hint(name)
  if misa.keybinding_hint then return misa.keybinding_hint(key_context,name) end
  local bank,slot=name:match("^option_(%d+)_(%d+)$"); bank,slot=tonumber(bank),tonumber(slot); if bank and banks[bank] and banks[bank][slot] then return "alt+"..banks[bank][slot] end
  for key,value in pairs(fallback) do if value==name then return key end end
end
local function first(panel,room) if room<=0 or #panel.items<=room then return 1 end; return math.max(1,math.min(panel.highlight-math.floor(room/2),#panel.items-room+1)) end
local function source_spec(id,context,db) local source=assert(sources[id],"unknown choice source: "..tostring(id)); local spec=source.items(context,db) or {}; if spec.items then return spec end; return {items=spec} end
local function apply_spec(session,spec,db)
  session.title=spec.title or session.title; session.purpose=spec.purpose or session.purpose; session.items=items(spec.items or {}); session.query=spec.query or ""; session.input_prefix=spec.input_prefix or ""; session.selected=spec.selected~=misa.json_null and spec.selected or nil; session.preference_scope=spec.preference_scope or session.preference_scope; session.tree={node=spec.tree_node or ""}; session.view_state={}
  session.view_ids=spec.views and clone(spec.views) or configured(session.purpose); session.custom_views=nil; return refresh(session,db)
end
local function push_narrow(session,narrow,db)
  session.stack[#session.stack+1]={title=session.title,purpose=session.purpose,items=session.items,query=session.query,input_prefix=session.input_prefix,selected=session.selected,preference_scope=session.preference_scope,tree=session.tree,view_ids=session.view_ids,custom_views=session.custom_views,view_state=session.view_state}
  local spec=narrow.source and source_spec(narrow.source,narrow.context,db) or narrow
  apply_spec(session,spec,db); return {consumed=true,narrowed=true}
end
local function pop_narrow(session,db)
  local frame=table.remove(session.stack); if not frame then return false end
  for key,value in pairs(frame) do session[key]=value end; refresh(session,db); return true
end
local function new(spec,db)
  assert(type(spec)=="table" and type(spec.title)=="string" and spec.title~="","choice session requires a title")
  local session={stack={}}; apply_spec(session,spec,db)
  if spec.view_definitions then session.view_ids,session.custom_views={},{}; for index,definition in ipairs(spec.view_definitions) do session.view_ids[index]=assert(definition.id); session.custom_views[index]={id=definition.id,title=definition.title,items=items(definition.items or {})} end; refresh(session,db) end
  return session
end
local function row(value,focused,selected,hotkey) local description=value.description; if description==value.label or description==value.id or description==tostring(value.value) then description=nil end; return {id=value.id,value=value.value,marker=focused and ">" or (selected and "✓" or " "),hotkey=hotkey,label=value.label,description=description,selected=selected,active=focused,preview=value.preview,path=value.path} end
return {setup=function(context)
  misa.choice_config=type(context.config)=="table" and type(context.config.choices)=="table" and context.config.choices or {}
  local declarations={accept={"enter","tab"},cancel={"escape","ctrl_c","ctrl_d","eof"},previous={"arrow_up"},next={"arrow_down"},cycle={"arrow_right"},cycle_previous={"arrow_left","alt+p"},favorite={"alt+v"},replace_view={"alt+/"},open_overlay={"alt+space"}}
  for name,keys in pairs(declarations) do misa.reg_keybinding({context=key_context,action=name,default=keys}) end; for bank,keys in ipairs(banks) do for slot,key in ipairs(keys) do misa.reg_keybinding({context=key_context,action="option_"..bank.."_"..slot,default={"alt+"..key}}) end end
  misa.reg_choice_view=function(id,view) assert(not sealed and type(id)=="string" and id~="" and not views[id],"invalid or duplicate choice view"); views[id]=assert(view) end
  misa.reg_choice_source=function(id,source) assert(not sealed and type(id)=="string" and id~="" and not sources[id],"invalid or duplicate choice source"); assert(type(source)=="table" and type(source.items)=="function","choice source requires items"); sources[id]=source end
  misa.choice_source=function(id,context_value,db) return source_spec(id,context_value,db) end
  misa.reg_choice_view("all",{title="All"})
  misa.reg_choice_view("favorites",{title="Favorites",include=function(value,session,db)local p=preference(db,session.preference_scope,value.id); return p and p.favorite==true end})
  misa.reg_choice_view("frecency",{title="Recent",include=function(value,session,db)local p=preference(db,session.preference_scope,value.id); return p and (p.uses or 0)>0 end,order=function(a,b,session,db)local pa,pb=preference(db,session.preference_scope,a.id) or {},preference(db,session.preference_scope,b.id) or {}; return ((pa.last or 0)*1000000+(pa.uses or 0))>((pb.last or 0)*1000000+(pb.uses or 0)) end})
  misa.reg_choice_view("slash-prefix",{title="Tree",compatible=function(session)return session.purpose=="command-completion" or session.purpose=="models" end,project=function(session)return assert(misa.choice_tree_project,"choice_tree is required")(session,matches) end})
  misa.choice_session=new; misa.choice_refresh=refresh; misa.choice_action=action; misa.choice_hint=hint; misa.choice_first_index=first
  misa.choice_set_items=function(session,values,db) session.items=items(values); return refresh(session,db) end
  misa.choice_replace_view=function(session,id,db) assert(compatible(id,session),"incompatible choice view"); session.view_ids[1]=id; if session.custom_views then session.custom_views[1]=false end; return refresh(session,db) end
  misa.choice_registered_views=function(session)local result={}; for id,view in pairs(views) do if compatible(id,session) then result[#result+1]={id=id,value=id,display=view.title or id,search=id} end end; table.sort(result,function(a,b)return a.id<b.id end); return result end
  misa.choice_accept=function(session,value,db) if value.tree_prefix then session.tree.node=value.tree_prefix; session.query=""; refresh(session,db); return {consumed=true,tree_changed=true} end; if value.narrow then return push_narrow(session,value.narrow,db) end; return {consumed=true,accepted=value} end
  misa.choice_rows=function(session,db,hotkeys) refresh(session,db); local result={}; for bank,panel in ipairs(session.panels) do local rows={}; for index,value in ipairs(panel.items) do rows[index]=row(value,bank==1 and index==panel.highlight,value.value==session.selected,hotkeys and hotkeys[bank] and hotkeys[bank][index]) end; result[bank]={id=panel.id,title=panel.title,rows=rows} end; return result end
  misa.choice_hotkeys=function(session,visible,room)local result={}; for bank=1,math.min(visible or 0,#session.panels,#banks) do result[bank]={}; local panel=session.panels[bank]; local start=first(panel,room); for slot=1,math.min(room,9) do if panel.items[start+slot-1] then result[bank][start+slot-1]=hint("option_"..bank.."_"..slot) end end end; return result end
  misa.choice_positional=function(session,name,visible,room) for bank=1,math.min(visible or 0,#session.panels,#banks) do for slot=1,math.min(room or 0,9) do if name=="option_"..bank.."_"..slot then return session.panels[bank].items[first(session.panels[bank],room)+slot-1] end end end end
  misa.choice_input=function(session,event,db)
    refresh(session,db); local panel=session.panels[1]; local state=session.view_state[session.view_ids[1]]; local name=event.action
    if event.kind=="text" and type(event.text)=="string" then session.query=session.query..event.text
    elseif event.kind=="backspace" then if session.query~="" then session.query=pop_utf8(session.query) elseif session.tree.node~="" then session.tree.node=misa.choice_tree_parent(session.tree.node); return {consumed=true,tree_changed=true} elseif pop_narrow(session,db) then return {consumed=true,narrowed=true} else return {consumed=false} end
    elseif name=="previous" and #panel.items>0 then state.highlight=state.highlight<=1 and #panel.items or state.highlight-1
    elseif name=="next" and #panel.items>0 then state.highlight=state.highlight>=#panel.items and 1 or state.highlight+1
    elseif name=="cycle" then table.insert(session.view_ids,table.remove(session.view_ids,1))
    elseif name=="cycle_previous" then table.insert(session.view_ids,1,table.remove(session.view_ids))
    elseif name=="replace_view" then return {consumed=true,replace_view=true}
    elseif name=="open_overlay" then return {consumed=true,open_overlay=true}
    elseif name=="favorite" and session.preference_scope and panel.highlight>0 then return {consumed=true,favorite=panel.items[panel.highlight].id}
    elseif name=="cancel" then if pop_narrow(session,db) then return {consumed=true,narrowed=true} end; return {consumed=true,cancelled=true}
    elseif name=="accept" and panel.highlight>0 then return misa.choice_accept(session,panel.items[panel.highlight],db)
    else return {consumed=false} end
    refresh(session,db); return {consumed=true,tree_changed=session.tree.node~=""}
  end
  misa.reg_interceptor({id="choices/seal",before=function(tx)if tx.event.type=="app/start" then sealed=true end; return tx end})
end}
