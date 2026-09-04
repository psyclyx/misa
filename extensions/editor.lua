-- Multiline editor state, command completion, and input transitions.
local function previous_cursor(text,cursor)
  if cursor==0 then return 0 end; local previous=cursor-1
  while previous>0 do local byte=text:byte(previous+1); if byte<128 or byte>=192 then break end; previous=previous-1 end; return previous
end
local function next_cursor(text,cursor)
  if cursor>=#text then return #text end; local next=cursor+1
  while next<#text do local byte=text:byte(next+1); if byte<128 or byte>=192 then break end; next=next+1 end; return next
end
local function state(db)
  db.editor=db.editor or {}; local editor=db.editor; editor.text=type(editor.text)=="string" and editor.text or ""; editor.busy=editor.busy==true
  if type(editor.cursor)~="number" or editor.cursor%1~=0 or editor.cursor<0 or editor.cursor>#editor.text then editor.cursor=#editor.text end; return editor
end
local function completion_matches(input,db)
  local matches={}; if input:sub(1,1)~="/" then return matches end
  local command_name,prefix=input:match("^(%S+)%s+(.*)$")
  if command_name then local command=misa.command(command_name); if not command then return matches end
    for _,candidate in ipairs(misa.command_completions(command,prefix,db)) do matches[#matches+1]={text=command.name.." "..candidate.value,label=candidate.label or candidate.value,description=candidate.description or ""} end; return matches
  end
  local choices={}; for _,command in ipairs(misa.commands()) do choices[#choices+1]={value=command.name,label=command.name,description=command.description} end
  if misa.fuzzy_choices then choices=misa.fuzzy_choices(choices,input) end
  for _,choice in ipairs(choices) do if misa.fuzzy_choices or choice.value:sub(1,#input)==input then matches[#matches+1]={text=choice.value,label=choice.label,description=choice.description} end end; return matches
end
local function command_input(text) local name,args=text:match("^(%S+)%s*(.-)%s*$"); return name and misa.command(name),args end
local function position(text,cursor)
  local prefix=text:sub(1,cursor); local row,start=1,0; for index in prefix:gmatch("()\n") do row,start=row+1,index end; return row,cursor-start
end
return {setup=function(context)
  local config=type(context.config)=="table" and context.config.ui or nil; local plain_prompt=type(config)=="table" and config.plain_prompt==true
  misa.editor_projection=function(db)
    local editor=assert(db.editor,"editor state is not initialized"); local row,byte=position(editor.text,editor.cursor); local matches=completion_matches(editor.completion_prefix or editor.text,db)
    return {busy=editor.busy,row=row,byte=byte,input=misa.render_component(db,"editor.input",{text=editor.text}).lines,
      completions=misa.render_component(db,"editor.completions",{matches=matches,active=editor.completion_prefix and editor.completion_index or nil}).lines}
  end
  misa.reg_event("app/start",function(db,_,cofx)
    state(db); if #cofx.argv~=0 then return {db=db} end; local fx={}
    if not cofx.terminal.interactive and plain_prompt then fx[#fx+1]={type="view/commit",lines={{spans={{text="misa> enter a prompt:",style="plain"}}}}} end
    fx[#fx+1]={type="terminal/read"}; return {db=db,fx=fx}
  end)
  misa.reg_event("agent/status",function(db,event) state(db).busy=event.status~="ready"; return {db=db} end)
  misa.reg_event("agent/unavailable",function(db,event,cofx) return {db=db,fx={{type="dispatch",event={type="transcript/harness",text=event.message,level="error"}},cofx.terminal.interactive and {type="terminal/read"} or {type="app/quit"}}} end)
  misa.reg_event("agent/completed",function(db,event,cofx) if event.exit then return {db=db,fx={{type="app/quit"}}} end return {db=db,fx={cofx.terminal.interactive and {type="dispatch",event={type="ui/redraw"}} or {type="terminal/read"}}} end)
  misa.reg_event("ui/redraw",function(db) return {db=db,fx={{type="terminal/read"}}} end)
  misa.reg_event("terminal/input",function(db,event)
    local editor=state(db)
    if editor.busy then
      if event.kind=="ctrl_c" then editor.text,editor.cursor,editor.completion_prefix,editor.completion_index="",0,nil,nil; return {db=db,fx={{type="dispatch",event={type="agent/cancel-active"}},{type="terminal/read"}}} end
      return {db=db,fx={{type="terminal/read"}}}
    end
    if event.kind=="text" then editor.text=editor.text:sub(1,editor.cursor)..event.text..editor.text:sub(editor.cursor+1); editor.cursor=editor.cursor+#event.text; editor.completion_prefix,editor.completion_index=nil,nil
    elseif event.kind=="backspace" then local p=previous_cursor(editor.text,editor.cursor); editor.text=editor.text:sub(1,p)..editor.text:sub(editor.cursor+1); editor.cursor=p; editor.completion_prefix,editor.completion_index=nil,nil
    elseif event.kind=="tab" then local prefix=editor.completion_prefix or editor.text; local matches=completion_matches(prefix,db); if #matches>0 then editor.completion_index=editor.completion_index and (editor.completion_index%#matches+1) or 1; editor.completion_prefix=prefix; editor.text=matches[editor.completion_index].text; editor.cursor=#editor.text end
    elseif (event.kind=="arrow_up" or event.kind=="arrow_down") and editor.completion_prefix then local matches=completion_matches(editor.completion_prefix,db); if #matches>0 then local d=event.kind=="arrow_up" and -1 or 1; editor.completion_index=((editor.completion_index or 1)-1+d)%#matches+1; editor.text=matches[editor.completion_index].text; editor.cursor=#editor.text end
    elseif event.kind=="arrow_left" then editor.completion_prefix,editor.completion_index=nil,nil; editor.cursor=previous_cursor(editor.text,editor.cursor)
    elseif event.kind=="arrow_right" then editor.completion_prefix,editor.completion_index=nil,nil; editor.cursor=next_cursor(editor.text,editor.cursor)
    elseif event.kind=="escape" and editor.completion_prefix then editor.completion_prefix,editor.completion_index=nil,nil
    elseif event.kind=="enter" and editor.text~="" then local prompt=editor.text; editor.text,editor.cursor,editor.completion_prefix,editor.completion_index="",0,nil,nil; local command,args=command_input(prompt); return {db=db,fx={{type="dispatch",event=command and {type=command.event,command=command.name,arguments=args} or {type="agent/submit",prompt=prompt}}}}
    elseif event.kind=="ctrl_c" then editor.text,editor.cursor,editor.completion_prefix,editor.completion_index="",0,nil,nil
    elseif event.kind=="ctrl_d" then if editor.text=="" then return {db=db,fx={{type="app/quit"}}} elseif editor.cursor<#editor.text then editor.text=editor.text:sub(1,editor.cursor)..editor.text:sub(next_cursor(editor.text,editor.cursor)+1) end
    elseif event.kind=="eof" then return {db=db,fx={{type="app/quit"}}} end
    return {db=db,fx={{type="terminal/read"}}}
  end)
end}
