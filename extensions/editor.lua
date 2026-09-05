-- Multiline editor state. Text and cursor stay here; slash choices delegate all
-- filtering, navigation, view, and positional-key behavior to choices.lua.
local function previous_cursor(text,cursor) return misa.layout.previous_boundary(text,cursor) end
local function next_cursor(text,cursor) return misa.layout.next_boundary(text,cursor) end
local function state(db)
  db.editor=db.editor or {}; local editor=db.editor
  editor.text=type(editor.text)=="string" and editor.text or ""; editor.busy=editor.busy==true
  if type(editor.cursor)~="number" or editor.cursor%1~=0 or editor.cursor<0 or editor.cursor>#editor.text then editor.cursor=#editor.text
  else editor.cursor=misa.layout.boundary_at_or_before(editor.text,editor.cursor) end
  return editor
end
local function command_input(text) local name,args=text:match("^(%S+)%s*(.-)%s*$"); return name and misa.command(name),args end
local function command_items()
  local result={}; for _,command in ipairs(misa.commands()) do result[#result+1]={value=command.name,label=command.name,description=command.description,search=command.search} end
  return result
end
local function clear_choice(editor) editor.choice,editor.choice_kind,editor.choice_command,editor.choice_overlay=nil,nil,nil,nil end
local function sync_choice(editor,db)
  clear_choice(editor)
  if editor.cursor~=#editor.text or editor.text:sub(1,1)~="/" or editor.dismissed_choice==editor.text then return end
  local command_name,args=editor.text:match("^(%S+)%s+(.*)$")
  if command_name then
    local command=misa.command(command_name); if not command then return end
    local items=misa.command_completions(command,"",db); if #items==0 and args~="" then items=misa.command_completions(command,args,db) end
    if #items==0 then return end
    editor.choice=misa.choice_session({title=command.name:sub(2),purpose=command.choice_purpose or "command",items=items,query=args,selected=command.selected and command.selected(db) or nil,preference_scope=command.preference_scope},db)
    editor.choice_kind,editor.choice_command="argument",command.name
  else
    editor.choice=misa.choice_session({title="Commands",purpose="command-completion",items=command_items(),query=editor.text:sub(2),tree_node="/"},db)
    editor.choice_kind,editor.choice_command="command",nil
  end
end
local function choice_text(editor)
  if editor.choice_kind=="argument" then return editor.choice_command.." "..editor.choice.query end
  return editor.choice.tree.node..editor.choice.query
end
local function sync_text_from_choice(editor)
  editor.text=choice_text(editor); editor.cursor=#editor.text; editor.dismissed_choice=nil
end
local function accept_choice(editor,item)
  if editor.choice_kind=="argument" then editor.text=editor.choice_command.." "..item.value else editor.text=item.value end
  editor.cursor=#editor.text; editor.dismissed_choice=nil; clear_choice(editor)
end
local function highlighted_value(editor)
  local panel=editor.choice and editor.choice.panels[1]
  return panel and panel.highlight>0 and panel.items[panel.highlight].value or nil
end
local function submit_exact_choice(editor)
  if editor.choice_kind=="command" then return misa.command(editor.text)~=nil end
  if editor.choice_kind=="argument" then return editor.choice.query=="" or highlighted_value(editor)==editor.choice.query end
  return false
end
local function overlay_effect(editor,choose_view)
  editor.choice_sequence=(editor.choice_sequence or 0)+1
  local token="editor:"..editor.choice_sequence; editor.choice_overlay=token
  return {type="dispatch",event={type="picker/open",id="inline-choice",token=token,title=editor.choice.title,completion="editor/choice-selected",session=editor.choice,choose_view=choose_view==true}}
end
local function update_dynamic_items(editor,db)
  if editor.choice_kind~="argument" then return end
  local command=misa.command(editor.choice_command); if not command then return end
  local items=misa.command_completions(command,editor.choice.query,db)
  if #items==0 then items=misa.command_completions(command,"",db) end
  misa.choice_set_items(editor.choice,items,db)
end
local function visible_rows(editor,db,room)
  local hotkeys=misa.choice_hotkeys(editor.choice,1,room); local columns=misa.choice_rows(editor.choice,db,hotkeys)
  local rows=columns[1] and columns[1].rows or {}; local panel=editor.choice.panels[1]; local first=misa.choice_first_index(panel,room); local result={}
  for index=first,math.min(#rows,first+room-1) do result[#result+1]=rows[index] end
  return result
end

return {setup=function(context)
  assert(misa.choice_session and misa.layout and misa.layout.previous_boundary,"editor requires choices and layout")
  local config=type(context.config)=="table" and context.config.ui or nil; local plain_prompt=type(config)=="table" and config.plain_prompt==true
  misa.editor_projection=function(db,projection_context)
    local editor=assert(db.editor,"editor state is not initialized"); local rows={}
    local terminal=projection_context and projection_context.terminal
    local input=misa.render_component(db,"editor.input",{text=editor.text,cursor=editor.cursor},{columns=terminal and terminal.columns or 80})
    local room=terminal and misa.inline_choice_room and misa.inline_choice_room(db,terminal,#input.lines) or 5
    if editor.choice and not editor.choice_overlay then rows=visible_rows(editor,db,room) end
    return {busy=editor.busy,row=input.cursor.row,byte=input.cursor.byte,input=input.lines,completions=misa.render_component(db,"editor.completions",{rows=rows}).lines}
  end
  misa.reg_event("app/start",function(db,_,cofx)
    state(db); if #cofx.argv~=0 then return {db=db} end; local fx={}
    if not cofx.terminal.interactive and plain_prompt then fx[#fx+1]={type="view/commit",lines={{spans={{text="misa> enter a prompt:",style={foreground="default"}}}}}} end
    fx[#fx+1]={type="terminal/read"}; return {db=db,fx=fx}
  end)
  misa.reg_event("agent/status",function(db,event) state(db).busy=event.status~="ready"; return {db=db} end)
  misa.reg_event("agent/unavailable",function(db,event,cofx) return {db=db,fx={{type="dispatch",event={type="transcript/harness",text=event.message,level="error"}},cofx.terminal.interactive and {type="terminal/read"} or {type="app/quit"}}} end)
  misa.reg_event("agent/completed",function(db,event,cofx) if event.exit then return {db=db,fx={{type="app/quit"}}} end return {db=db,fx={cofx.terminal.interactive and {type="dispatch",event={type="ui/redraw"}} or {type="terminal/read"}}} end)
  misa.reg_event("ui/redraw",function(db) return {db=db,fx={{type="terminal/read"}}} end)
  misa.reg_event("editor/choice-selected",function(db,event)
    local editor=state(db); if event.picker~="inline-choice" or event.picker_token~=editor.choice_overlay or not editor.choice then return end
    editor.choice_overlay=nil
    if not event.cancelled then
      local accepted; for _,item in ipairs(editor.choice.items) do if item.value==event.value then accepted=item; break end end
      if accepted then accept_choice(editor,accepted) end
    end
    return {db=db,fx={{type="terminal/read"}}}
  end)
  misa.reg_event("terminal/input",function(db,event,cofx)
    local editor=state(db)
    if editor.busy then
      if event.kind=="ctrl_c" then editor.text,editor.cursor,editor.dismissed_choice="",0,nil; clear_choice(editor); return {db=db,fx={{type="dispatch",event={type="agent/cancel-active"}},{type="terminal/read"}}} end
      if cofx.terminal.interactive then return {db=db,fx={{type="terminal/read"}}} end
      return {db=db}
    end

    -- Whitespace changes a command-name completion into its argument session.
    if editor.choice and editor.choice_kind=="command" and event.kind=="text" and event.text:find("%s") then
      editor.text=editor.text:sub(1,editor.cursor)..event.text..editor.text:sub(editor.cursor+1); editor.cursor=editor.cursor+#event.text; editor.dismissed_choice=nil
      sync_choice(editor,db); return {db=db,fx={{type="terminal/read"}}}
    end

    local action=misa.choice_action(event)
    if event.kind=="enter" and submit_exact_choice(editor) then action=nil end
    if editor.choice and (action or event.kind=="text" or event.kind=="backspace") then
      local input_count=#misa.render_component(db,"editor.input",{text=editor.text,cursor=editor.cursor},{columns=cofx.terminal.columns}).lines
      local room=misa.inline_choice_room and misa.inline_choice_room(db,cofx.terminal,input_count) or 5
      local item=misa.choice_positional(editor.choice,action,room>0 and 1 or 0,room)
      local result=item and misa.choice_accept(editor.choice,item,db) or misa.choice_input(editor.choice,{kind=event.kind,text=event.text,action=action},db)
      if result.accepted then accept_choice(editor,result.accepted); return {db=db,fx={{type="terminal/read"}}}
      elseif result.open_overlay or result.replace_view then
        if misa.picker then return {db=db,fx={overlay_effect(editor,result.replace_view)}} end
      elseif result.tree_changed then sync_text_from_choice(editor); if editor.text=="" then clear_choice(editor) end; return {db=db,fx={{type="terminal/read"}}}
      elseif result.cancelled then editor.dismissed_choice=editor.text; clear_choice(editor); return {db=db,fx={{type="terminal/read"}}}
      elseif result.favorite then return {db=db,fx={{type="dispatch",event={type="preferences/toggle",scope=editor.choice.preference_scope,value=result.favorite}},{type="terminal/read"}}}
      elseif result.consumed then update_dynamic_items(editor,db); sync_text_from_choice(editor); if editor.text=="" then clear_choice(editor) end; return {db=db,fx={{type="terminal/read"}}} end
    end

    if event.kind=="text" then editor.text=editor.text:sub(1,editor.cursor)..event.text..editor.text:sub(editor.cursor+1); editor.cursor=editor.cursor+#event.text; editor.dismissed_choice=nil
    elseif event.kind=="backspace" then local previous=previous_cursor(editor.text,editor.cursor); editor.text=editor.text:sub(1,previous)..editor.text:sub(editor.cursor+1); editor.cursor=previous; editor.dismissed_choice=nil
    elseif event.kind=="arrow_left" then editor.cursor=previous_cursor(editor.text,editor.cursor); editor.dismissed_choice=nil
    elseif event.kind=="arrow_right" then editor.cursor=next_cursor(editor.text,editor.cursor); editor.dismissed_choice=nil
    elseif event.kind=="enter" and editor.text~="" then local prompt=editor.text; editor.text,editor.cursor,editor.dismissed_choice="",0,nil; clear_choice(editor); local command,args=command_input(prompt); return {db=db,fx={{type="dispatch",event=command and {type=command.event,command=command.name,arguments=args} or {type="agent/submit",prompt=prompt}}}}
    elseif event.kind=="ctrl_c" then editor.text,editor.cursor,editor.dismissed_choice="",0,nil; clear_choice(editor)
    elseif event.kind=="ctrl_d" then if editor.text=="" then return {db=db,fx={{type="app/quit"}}} elseif editor.cursor<#editor.text then editor.text=editor.text:sub(1,editor.cursor)..editor.text:sub(next_cursor(editor.text,editor.cursor)+1) end
    elseif event.kind=="eof" then return {db=db,fx={{type="app/quit"}}} end
    sync_choice(editor,db); return {db=db,fx={{type="terminal/read"}}}
  end)
end}
