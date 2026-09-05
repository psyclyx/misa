-- Command palette composed from registered commands and canonical recent
-- invocations. Argument completion is an ordinary narrowing source.
local function command_items(db)
  local result={}
  for _,command in ipairs(misa.commands()) do
    local narrow
    if command.completion or command.complete then narrow={source="command-arguments",context={command=command.name}} end
    result[#result+1]={id="command:"..command.name,value=command.name,display={label=command.name,description=command.description},search={command.name,command.description},path=command.name,preview={title=command.name,description=command.description},narrow=narrow,invocation=not narrow and command.name or nil}
  end
  for _,entry in ipairs(misa.command_recent(db)) do result[#result+1]={id="recent:"..entry.canonical,value=entry.canonical,display={label=entry.canonical,description="recent invocation"},search={entry.command,entry.arguments},path=entry.canonical,preview={title=entry.canonical,kind="recent"},invocation=entry.canonical} end
  return result
end
return {setup=function()
  assert(misa.reg_choice_source and misa.command_invocation,"omnipicker requires choices and commands")
  misa.reg_choice_source("omnipicker",{items=function(_,db)return {title="Commands",purpose="command-completion",items=command_items(db),views={"slash-prefix"},tree_node="/"} end})
  misa.reg_choice_source("command-arguments",{items=function(context,db)
    local command=assert(misa.command(context.command)); local result={}
    for index,candidate in ipairs(misa.command_completions(command,"",db)) do
      local canonical=misa.command_canonical(command.name,candidate.value)
      result[#result+1]={id=command.name..":"..(candidate.id or candidate.value or index),value=canonical,display=candidate.display or {label=candidate.label or candidate.value,description=candidate.description},search=candidate.search,path=tostring(candidate.value),preview=candidate.preview,invocation=canonical}
    end
    return {title=command.name:sub(2),purpose=command.choice_purpose or "command",items=result,input_prefix=command.name.." ",selected=command.selected and misa.command_canonical(command.name,command.selected(db)),preference_scope=command.preference_scope}
  end})
  misa.omnipicker_session=function(db,query)
    local spec=misa.choice_source("omnipicker",nil,db); spec.query=query or ""; return misa.choice_session(spec,db)
  end
  misa.reg_keybinding({context="global",action="open_omnipicker",default={"alt+/"}})
  misa.reg_event("omnipicker/open",function(db)
    db.omnipicker_sequence=(db.omnipicker_sequence or 0)+1; local token="omnipicker:"..db.omnipicker_sequence
    return {db=db,fx={{type="dispatch",event={type="picker/open",id="omnipicker",token=token,title="Commands",completion="omnipicker/selected",session=misa.omnipicker_session(db,""),preference_scope="commands"}}}}
  end)
  misa.reg_event("omnipicker/selected",function(db,event)
    if event.picker~="omnipicker" then return end; if event.cancelled then return {db=db,fx={{type="terminal/read"}}} end
    local invocation=assert(misa.command_invocation(tostring(event.value)),"invalid command palette invocation")
    return {db=db,fx={{type="dispatch",event=invocation}}}
  end)
  misa.reg_interceptor({id="omnipicker/global",before=function(tx)
    if tx.event.type=="terminal/input" and not tx.db.picker and misa.keybinding_action("global",tx.event)=="open_omnipicker" then tx.event={type="omnipicker/open"} end; return tx
  end})
end}
