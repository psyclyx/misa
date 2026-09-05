-- Canonical command invocation and generic recent replay. Command execution has
-- one normalization path whether input was typed, picked, or replayed.
local function trim(value) return (tostring(value or ""):match("^%s*(.-)%s*$")) end
local function canonical(name,arguments) local args=trim(arguments); return name..(args~="" and (" "..args) or "") end
return {setup=function()
  misa.command_invocation=function(text)
    assert(type(text)=="string","command invocation must be a string"); local name,args=text:match("^(%S+)%s*(.-)%s*$"); local command=name and misa.command(name)
    if not command then return nil end
    args=trim(args); return {type=command.event,command=command.name,arguments=args,canonical=canonical(command.name,args),normalized_command=true}
  end
  misa.command_canonical=canonical
  misa.command_recent=function(db) local result={}; for _,entry in ipairs(db.commands and db.commands.recent or {}) do result[#result+1]=entry end; return result end
  misa.reg_interceptor({id="commands/normalize",before=function(tx)
    local event=tx.event; if type(event.command)~="string" then return tx end
    local command=misa.command(event.command); if not command then return tx end
    local args=trim(event.arguments)
    if args=="" and (command.completion or command.complete) and not event.resumed_choice then tx.event={type="choices/command-open",command=command.name}; return tx end
    event.arguments=args; event.canonical=canonical(command.name,args); event.normalized_command=true
    tx.db.commands=tx.db.commands or {recent={}}; local recent=tx.db.commands.recent; for index,entry in ipairs(recent) do if entry.canonical==event.canonical then table.remove(recent,index); break end end
    table.insert(recent,1,{canonical=event.canonical,command=command.name,arguments=args}); while #recent>20 do table.remove(recent) end
    return tx
  end})
  misa.reg_event("choices/command-open",function(db,event)
    local command=assert(misa.command(event.command)); db.choice_commands=db.choice_commands or {sequence=0,pending={}}; local state=db.choice_commands; state.sequence=state.sequence+1
    local token="command:"..state.sequence; state.pending[token]={command=command.name}
    return {db=db,fx={{type="dispatch",event={type="picker/open",id="command-choice",token=token,title=command.name:sub(2),completion="choices/command-selected",purpose=command.choice_purpose or "command",items=misa.command_completions(command,"",db),selected=command.selected and command.selected(db),preference_scope=command.preference_scope or ("command:"..command.name)}}}}
  end)
  misa.reg_event("choices/command-selected",function(db,event)
    local pending=db.choice_commands and db.choice_commands.pending[event.picker_token]; if event.picker~="command-choice" or not pending then return end
    db.choice_commands.pending[event.picker_token]=nil; if event.cancelled then return {db=db,fx={{type="terminal/read"}}} end
    local invocation=assert(misa.command_invocation(pending.command.." "..tostring(event.value))); invocation.resumed_choice=true
    return {db=db,fx={{type="dispatch",event=invocation}}}
  end)
end}
