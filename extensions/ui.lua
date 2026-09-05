-- Root composition only. Feature state and viewport extraction remain owned by
-- editor, messages, picker layers, and status.
local function append(target,source,limit)
  for _,line in ipairs(source or {}) do if not limit or #target<limit then target[#target+1]=line end end
end
local function slice(lines,first,count)
  local result={}; for index=math.max(1,first),math.min(#lines,first+count-1) do result[#result+1]=lines[index] end; return result
end
local function bound_frame(lines,columns,cursor)
  columns=math.max(1,columns); local result={}
  for _,line in ipairs(lines) do local remaining,spans=columns,{}
    for _,source in ipairs(line.spans or {}) do if remaining>0 then local text,_,used=misa.layout.take(source.text or "",remaining); spans[#spans+1]={text=text,style=source.style}; remaining=math.max(0,remaining-used) end end
    result[#result+1]={spans=spans}
  end
  if cursor then local bytes=0; for _,item in ipairs(result[cursor.row] and result[cursor.row].spans or {}) do bytes=bytes+#(item.text or "") end; cursor.byte=math.min(cursor.byte,bytes) end
  return {lines=result,cursor=cursor}
end
return {setup=function()
  misa.picker_available_lines=function(db,terminal)
    local height=math.max(0,terminal.lines); local header=misa.render_component(db,"root.header",{}).lines
    return math.max(0,height-(height>=4 and math.min(2,#header) or 0))
  end
  -- Inline choices use the same window for rendering and positional-key
  -- resolution; root composition is the sole owner of this geometry.
  misa.inline_choice_room=function(db,terminal,input_count)
    local height=math.max(0,terminal.lines); if height==0 then return 0 end
    local header=misa.render_component(db,"root.header",{}).lines; local header_count=height>=4 and math.min(2,#header) or 0
    local status=misa.status_projection and misa.status_projection(db,{columns=terminal.columns}) or {}; local status_count=(#status>0 and height>=2) and 1 or 0
    local editor_budget=math.min(input_count,math.max(1,math.floor(height/2)),math.max(1,height-header_count-status_count))
    local remaining=math.max(0,height-header_count-status_count-editor_budget)
    return math.max(0,math.min(5,math.floor(remaining/3)))
  end
  misa.reg_view(function(db,cofx)
    local height=math.max(0,cofx.terminal.lines); if height==0 then return {lines={}} end
    local header=misa.render_component(db,"root.header",{}).lines; local header_count=height>=4 and math.min(2,#header) or 0
    local layer_cofx={terminal=cofx.terminal,available_lines=math.max(0,height-header_count)}
    for _,layer in ipairs(misa.view_layers(db,layer_cofx)) do if layer.exclusive then
      local lines={}; append(lines,header,header_count); local offset=#lines; append(lines,layer.lines,height)
      local cursor=layer.cursor and {row=math.min(height,offset+layer.cursor.row),byte=layer.cursor.byte} or nil
      return bound_frame(lines,cofx.terminal.columns,cursor)
    end end
    local status=misa.status_projection and misa.status_projection(db,{columns=cofx.terminal.columns}) or {}; local status_count=(#status>0 and height>=2) and 1 or 0
    local editor=misa.editor_projection and misa.editor_projection(db,{terminal=cofx.terminal}) or {busy=true,row=1,byte=0,input={},completions={}}
    local editor_budget=math.min(#editor.input,math.max(1,math.floor(height/2)),math.max(1,height-header_count-status_count))
    local editor_first=math.max(1,math.min(editor.row-math.floor(editor_budget/2),#editor.input-editor_budget+1)); local editor_lines=slice(editor.input,editor_first,editor_budget)
    local remaining=math.max(0,height-header_count-status_count-#editor_lines); local completion_count=math.min(#editor.completions,5,math.floor(remaining/3)); remaining=remaining-completion_count
    local transcript=misa.transcript_window and misa.transcript_window(db,{interactive=true,columns=cofx.terminal.columns},remaining) or {}
    local lines={}; append(lines,header,header_count); append(lines,transcript); local input_offset=#lines; append(lines,editor_lines); append(lines,editor.completions,#lines+completion_count); append(lines,status,status_count>0 and #lines+status_count or #lines)
    local cursor; if not editor.busy then cursor={row=math.max(1,math.min(height,input_offset+(editor.row-editor_first+1))),byte=editor.byte+((editor.row==1) and 2 or 0)} end
    return bound_frame(lines,cofx.terminal.columns,cursor)
  end)
end}
