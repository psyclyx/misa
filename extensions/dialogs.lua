-- Generic correlated in-UI interactions. This module owns lifecycle and input;
-- producers supply semantic data and never select a visual implementation.
local function copy_actions(actions)
  local result={}
  for index,action in ipairs(actions or {}) do
    assert(type(action)=="table" and type(action.id)=="string" and action.id~="","invalid dialog action")
    result[index]={id=action.id,label=action.label or action.id,primary=action.primary==true}
  end
  return result
end
local function finish(state,action,cancelled)
  return {{type="dispatch",event={type=state.completion,id=state.id,correlation=state.correlation,
    action=action,value=state.input or "",cancelled=cancelled==true}}}
end
return {setup=function()
  misa.dialogs=true
  misa.reg_event("dialog/open",function(db,event)
    assert(not db.dialog,"a dialog is already open")
    assert(type(event.id)=="string" and event.id~="" and type(event.correlation)=="string" and event.correlation~="","invalid dialog identity")
    assert(type(event.completion)=="string" and event.completion~="","invalid dialog completion")
    local kind=event.kind or "modal"; assert(kind=="modal" or kind=="progress" or kind=="alert","invalid dialog kind")
    db.dialog={id=event.id,correlation=event.correlation,completion=event.completion,kind=kind,title=event.title or "",
      message=event.message or "",url=event.url,code=event.code,progress=event.progress,hints=event.hints or {},
      cancellable=event.cancellable==true,input_enabled=event.input==true,input=event.initial or "",actions=copy_actions(event.actions)}
    return {db=db,fx={{type="terminal/read"}}}
  end)
  misa.reg_event("dialog/update",function(db,event)
    local state=db.dialog; if not state or state.id~=event.id or state.correlation~=event.correlation then return end
    for _,name in ipairs({"kind","title","message","url","code","progress","cancellable"}) do if event[name]~=nil then state[name]=event[name] end end
    if event.hints~=nil then state.hints=event.hints end
    if event.actions~=nil then state.actions=copy_actions(event.actions) end
    if event.input~=nil then state.input_enabled=event.input==true end
    return {db=db,fx={{type="terminal/read"}}}
  end)
  misa.reg_event("dialog/close",function(db,event)
    local state=db.dialog; if not state or state.id~=event.id or (event.correlation and state.correlation~=event.correlation) then return end
    db.dialog=nil; return {db=db}
  end)
  misa.reg_interceptor({id="dialogs/input",before=function(tx)
    if tx.event.type=="terminal/input" and tx.db.dialog then tx.event={type="dialog/input",kind=tx.event.kind,text=tx.event.text} end
    return tx
  end})
  misa.reg_event("dialog/input",function(db,event)
    local state=assert(db.dialog)
    if (event.kind=="escape" or event.kind=="ctrl_c" or event.kind=="ctrl_d" or event.kind=="eof") and state.cancellable then
      db.dialog=nil; return {db=db,fx=finish(state,"cancel",true)}
    end
    if state.input_enabled then
      if event.kind=="text" then state.input=state.input..(event.text or "")
      elseif event.kind=="backspace" then local last=#state.input-1; while last>0 and state.input:byte(last+1)>=128 and state.input:byte(last+1)<192 do last=last-1 end; state.input=state.input:sub(1,last)
      elseif event.kind=="enter" then local action=(state.actions[1] and state.actions[1].id) or "submit"; db.dialog=nil; return {db=db,fx=finish(state,action,false)} end
    elseif event.kind=="enter" and #state.actions>0 then local action=state.actions[1].id; db.dialog=nil; return {db=db,fx=finish(state,action,false)} end
    return {db=db,fx={{type="terminal/read"}}}
  end)
end}
