-- Canonical transcript models and projection. Interactive output is composed
-- only from db.messages.transcript by the root managed view.
local function lower(value) return tostring(value):lower() end
-- Transcript values cross the final trust boundary here. Keep line feeds, turn
-- tabs/CR into stable text, strip terminal controls, and repair malformed UTF-8
-- before any component can project the value into a semantic view.
local function printable_text(value)
  local out, i, length = {}, 1, #value
  local function continuation(byte) return byte and byte >= 0x80 and byte <= 0xbf end
  while i <= length do
    local byte = value:byte(i)
    if byte == 0x1b then
      local next_byte = value:byte(i + 1)
      if next_byte == 0x5b then
        i = i + 2
        while i <= length do local current=value:byte(i); i=i+1; if current>=0x40 and current<=0x7e then break end end
      elseif next_byte == 0x5d then
        i = i + 2
        while i <= length do
          local current = value:byte(i)
          if current == 0x07 then i=i+1; break end
          if current == 0x1b and value:byte(i+1) == 0x5c then i=i+2; break end
          i=i+1
        end
      else i=i+1 end
    elseif byte == 0x09 then out[#out+1]=" "; i=i+1
    elseif byte == 0x0d then out[#out+1]="\n"; i=i+1; if value:byte(i)==0x0a then i=i+1 end
    elseif byte == 0x0a then out[#out+1]="\n"; i=i+1
    elseif byte < 0x20 or byte == 0x7f then i=i+1
    else
      local width = byte < 0x80 and 1 or (byte >= 0xc2 and byte <= 0xdf and 2 or (byte >= 0xe0 and byte <= 0xef and 3 or (byte >= 0xf0 and byte <= 0xf4 and 4 or 0)))
      local b2, b3, b4 = value:byte(i+1), value:byte(i+2), value:byte(i+3)
      local valid = width == 1 or
        (width == 2 and continuation(b2)) or
        (width == 3 and continuation(b2) and continuation(b3) and not (byte==0xe0 and b2<0xa0) and not (byte==0xed and b2>0x9f)) or
        (width == 4 and continuation(b2) and continuation(b3) and continuation(b4) and not (byte==0xf0 and b2<0x90) and not (byte==0xf4 and b2>0x8f))
      if valid then
        if not (byte==0xc2 and b2>=0x80 and b2<=0x9f) then out[#out+1]=value:sub(i,i+width-1) end
        i=i+width
      else out[#out+1]="�"; i=i+1 end
    end
  end
  return table.concat(out)
end
local function truncate_text(value, limit)
  if #value <= limit then return value end
  local boundary = limit
  while boundary > 0 and value:byte(boundary + 1) and value:byte(boundary + 1) >= 0x80 and value:byte(boundary + 1) <= 0xbf do boundary=boundary-1 end
  return value:sub(1,boundary) .. "… [truncated " .. tostring(#value - boundary) .. " bytes]"
end
local function copy_structural(value, policy, depth, key)
  if key and policy.redact[lower(key)] then return "[redacted]" end
  local kind = type(value)
  if kind == "string" then return truncate_text(printable_text(value), policy.max_string) end
  if kind ~= "table" then return value end
  if depth >= policy.max_depth then return "[truncated structure]" end
  local result, count = {}, 0
  for child_key, child in pairs(value) do
    count = count + 1; if count > policy.max_items then result["…"] = "[truncated items]"; break end
    result[child_key] = copy_structural(child, policy, depth + 1, child_key)
  end
  return result
end
local function describe(value, depth)
  local kind = type(value)
  if kind == "string" then return printable_text(value) end
  if kind ~= "table" then return printable_text(tostring(value)) end
  if depth > 3 then return "{…}" end
  local values = {}; for key, child in pairs(value) do values[#values + 1] = printable_text(tostring(key)) .. "=" .. describe(child, depth + 1) end
  table.sort(values); return "{" .. table.concat(values, ", ") .. "}"
end
local function noninteractive_commit(db, role, model, cofx, markdown)
  if cofx.terminal.interactive or not misa.render_component then return {} end
  local rendered=misa.render_component(db,role,model,{interactive=false,columns=cofx.terminal.columns,markdown=markdown})
  if #(rendered.lines or {})==0 then return {} end
  return {{type="view/commit",lines=rendered.lines}}
end
local function clock(cofx)
  local value=assert(cofx.clock,"native clock coeffect is missing")
  return value.wall_ms,value.monotonic_ms
end
local function append_response(db,id,role,cofx,status)
  local state=assert(db.messages,"message state is not initialized")
  assert(not state.by_response[id],"duplicate transcript response: "..tostring(id))
  local wall,mono=clock(cofx); local response={id=id,role=role,status=status or "streaming",started_wall_ms=wall,started_monotonic_ms=mono,block_start=#state.blocks+1,block_count=0}
  state.responses[#state.responses+1]=response; state.by_response[id]=#state.responses; state.scroll=0; return response
end
local function response(db,id)
  local state=db.messages; local index=state and state.by_response[id]; return index and state.responses[index] or nil
end
local function append_block(db,response_model,block)
  local state=db.messages; block.response_id=response_model.id; block.role=response_model.role; block.started_wall_ms=response_model.started_wall_ms
  state.blocks[#state.blocks+1]=block; state.transcript=state.blocks; response_model.block_count=response_model.block_count+1; state.scroll=0; return block
end
local function find_block(db,response_model,id)
  local blocks=db.messages.blocks
  for index=response_model.block_start,response_model.block_start+response_model.block_count-1 do if blocks[index] and blocks[index].id==id then return blocks[index] end end
end
local function append_delta(block,value,policy)
  value=printable_text(tostring(value or "")); if value=="" or block.truncated then return end
  local remaining=policy.max_string-(block.byte_count or 0)
  if remaining<=0 then block.chunks[#block.chunks+1]="… [truncated]"; block.truncated=true; return end
  local piece=value
  if #piece>remaining then piece=truncate_text(piece,remaining); block.truncated=true end
  block.chunks[#block.chunks+1]=piece; block.byte_count=(block.byte_count or 0)+math.min(#value,remaining)
end
local function finish_block(block, event, policy)
  if block.chunks then block.text=table.concat(block.chunks); block.chunks=nil end
  if event and event.arguments~=nil then block.arguments=copy_structural(event.arguments,policy,0) end
  if event and event.name~=nil then block.name=printable_text(tostring(event.name)) end
  if event and event.call_id~=nil then block.call_id=printable_text(tostring(event.call_id)) end
  if block.arguments~=nil then block.argument_chunks=nil; block.argument_text=nil
  elseif block.argument_chunks then block.argument_text=table.concat(block.argument_chunks); block.argument_chunks=nil end
  block.argument_bytes=nil
  block.streaming=false
end
local function timestamp(ms)
  local seconds=math.floor((tonumber(ms) or 0)/1000)%86400
  return string.format("%02d:%02d:%02d",math.floor(seconds/3600),math.floor(seconds/60)%60,seconds%60)
end

return { setup=function(context)
  local config=type(context.config)=="table" and context.config.messages or nil; config=type(config)=="table" and config or {}
  local markdown=config.plain~=true and config.markdown~=false
  local policy={max_string=config.max_string or 4000,max_items=config.max_items or 64,max_depth=config.max_depth or 8,redact={}}
  assert(type(policy.max_string)=="number" and policy.max_string>0,"messages.max_string must be positive")
  for _,key in ipairs(config.redact_keys or {"authorization","api_key","password","secret","token"}) do policy.redact[lower(key)]=true end
  misa.reg_keybinding({context="global",action="toggle_verbose",default={"alt+t"}})
  misa.reg_keybinding({context="global",action="transcript_up",default={"page_up","alt+k"}})
  misa.reg_keybinding({context="global",action="transcript_down",default={"page_down","alt+j"}})
  if misa.reg_indicator then misa.reg_indicator({id="transcript-detail",label="detail",icon="≡",hotkey={context="global",action="toggle_verbose"},value=function(db) return (db.messages or {}).verbose and "verbose" or "summary" end}) end
  misa.reg_command({name="/verbose",description="Toggle thinking and tool transcript detail",event="messages/toggle-verbose"})
  misa.reg_event("app/start",function(db) db.messages={responses={},blocks={},transcript={},by_response={},verbose=config.verbose==true,scroll=0,next_id=0}; db.messages.transcript=db.messages.blocks; return {db=db} end)
  misa.reg_event("messages/toggle-verbose",function(db) db.messages.verbose,db.messages.scroll=not db.messages.verbose,0; return {db=db,fx={{type="dispatch",event={type="ui/redraw"}}}} end)
  misa.reg_event("messages/scroll",function(db,event) db.messages.scroll=math.max(0,db.messages.scroll+event.delta); return {db=db,fx={{type="terminal/read"}}} end)
  misa.reg_interceptor({id="messages/global-keys",before=function(tx)
    if tx.event.type=="terminal/input" and not tx.db.picker and misa.keybinding_action then local action=misa.keybinding_action("global",tx.event)
      if action=="toggle_verbose" then tx.event={type="messages/toggle-verbose"}
      elseif action=="transcript_up" then tx.event={type="messages/scroll",delta=math.max(1,math.floor(tx.cofx.terminal.lines/2))}
      elseif action=="transcript_down" then tx.event={type="messages/scroll",delta=-math.max(1,math.floor(tx.cofx.terminal.lines/2))} end
    end; return tx
  end})
  misa.messages_projection=function(db) return {verbose=(db.messages or {}).verbose==true} end
  misa.transcript_projection=function(db,render_context)
    local context_copy={}; for key,value in pairs(render_context or {}) do context_copy[key]=value end; context_copy.markdown=markdown
    local result,state={},assert(db.messages,"message state is not initialized")
    for _,source in ipairs(state.blocks) do
      local model={}; for key,value in pairs(source) do model[key]=value end
      if model.chunks then model.text=table.concat(model.chunks) end
      model.timestamp=timestamp(model.started_wall_ms)
      local owner=response(db,model.response_id)
      if owner and owner.metadata_block_id==model.id then model.tokens_per_second=owner.tokens_per_second end
      local role
      if model.kind=="user" then role="transcript.user"
      elseif model.kind=="assistant" then role="transcript.assistant"
      elseif model.kind=="thinking" then role=state.verbose and "transcript.thinking" or "transcript.thinking_collapsed"; model.summary=model.streaming and "streaming" or "summary"
      elseif model.kind=="tool_call" then role="transcript.tool_call"; model.detail=state.verbose and (model.arguments and describe(model.arguments,0) or printable_text(model.argument_text or table.concat(model.argument_chunks or {}))) or (model.streaming and "streaming" or "summary")
      elseif model.kind=="tool_result" then role="transcript.tool_result"; model.collapsed=not state.verbose
      elseif model.kind=="harness" then role="transcript.harness" end
      if role then local rendered=misa.render_component(db,role,model,context_copy); for _,line in ipairs(rendered.lines or {}) do result[#result+1]=line end end
    end
    return result
  end
  misa.transcript_window=function(db,render_context,available_lines)
    local lines=misa.transcript_projection(db,render_context); local room=math.max(0,math.floor(available_lines or 0)); if room==0 then return {} end
    local scroll=math.min((db.messages or {}).scroll or 0,math.max(0,#lines-room)); local first=math.max(1,#lines-room-scroll+1); local result={}
    for index=first,math.min(#lines,first+room-1) do result[#result+1]=lines[index] end; return result
  end
  misa.reg_event("transcript/reset",function(db) db.messages.responses,db.messages.blocks,db.messages.by_response={}, {}, {}; db.messages.transcript=db.messages.blocks; db.messages.scroll=0; return {db=db} end)

  misa.reg_event("transcript/response-start",function(db,event,cofx)
    assert(type(event.response_id)=="string" and event.response_id~="","response ID must be nonempty")
    append_response(db,event.response_id,event.role or "assistant",cofx,"streaming"); return {db=db}
  end)
  misa.reg_event("transcript/block-start",function(db,event)
    local owner=assert(response(db,event.response_id),"unknown transcript response"); assert(owner.status=="streaming","response is finalized")
    assert(type(event.block_id)=="string" and event.block_id~="" and not find_block(db,owner,event.block_id),"invalid transcript block ID")
    local kind=assert(event.kind,"transcript block kind is missing"); local block={id=event.block_id,kind=kind,streaming=true,interrupted=false}
    if kind=="assistant" or kind=="thinking" then block.chunks={}; block.byte_count=0
    elseif kind=="tool_call" then block.name=printable_text(tostring(event.name or "tool")); block.call_id=event.call_id; block.argument_chunks={}; block.argument_bytes=0
    else error("unsupported streaming transcript block: "..tostring(kind)) end
    append_block(db,owner,block); return {db=db}
  end)
  misa.reg_event("transcript/block-delta",function(db,event)
    local owner=assert(response(db,event.response_id),"unknown transcript response"); local block=assert(find_block(db,owner,event.block_id),"unknown transcript block")
    assert(block.streaming,"transcript block is finalized")
    if block.kind=="assistant" or block.kind=="thinking" then append_delta(block,event.text,policy)
    else
      if event.name~=nil then block.name=printable_text(tostring(event.name)) end; if event.call_id~=nil then block.call_id=printable_text(tostring(event.call_id)) end
      if event.arguments_json_delta~=nil and not block.arguments_truncated then
        local value=printable_text(tostring(event.arguments_json_delta)); local remaining=policy.max_string-(block.argument_bytes or 0)
        if remaining<=0 then block.argument_chunks[#block.argument_chunks+1]="… [truncated]"; block.arguments_truncated=true
        elseif #value>remaining then block.argument_chunks[#block.argument_chunks+1]=truncate_text(value,remaining); block.argument_bytes=policy.max_string; block.arguments_truncated=true
        else block.argument_chunks[#block.argument_chunks+1]=value; block.argument_bytes=(block.argument_bytes or 0)+#value end
      end
      if event.arguments~=nil then block.arguments=copy_structural(event.arguments,policy,0) end
    end
    return {db=db}
  end)
  misa.reg_event("transcript/block-end",function(db,event)
    local owner=assert(response(db,event.response_id),"unknown transcript response"); finish_block(assert(find_block(db,owner,event.block_id),"unknown transcript block"),event,policy); return {db=db}
  end)
  misa.reg_event("transcript/response-end",function(db,event,cofx)
    local owner=assert(response(db,event.response_id),"unknown transcript response"); local _,now=clock(cofx)
    for index=owner.block_start,owner.block_start+owner.block_count-1 do local block=db.messages.blocks[index]; if block.streaming then finish_block(block,nil,policy) end end
    owner.status="complete"; owner.completed_monotonic_ms=now; owner.elapsed_ms=math.max(0,now-owner.started_monotonic_ms)
    local output=type(event.usage)=="table" and event.usage.output_tokens or nil
    if type(output)=="number" and output>=0 and owner.elapsed_ms>0 then owner.output_tokens=output; owner.tokens_per_second=output*1000/owner.elapsed_ms end
    local fx={}; if owner.role=="assistant" then
      local text,last_text={}
      for i=owner.block_start,owner.block_start+owner.block_count-1 do
        local block=db.messages.blocks[i]
        if block.kind=="assistant" then text[#text+1]=block.text or ""; last_text=block end
      end
      if last_text then owner.metadata_block_id=last_text.id end
      if #text>0 then local committed={text=table.concat(text,""),timestamp=timestamp(owner.started_wall_ms),tokens_per_second=owner.tokens_per_second}; for _,effect in ipairs(noninteractive_commit(db,"transcript.assistant",committed,cofx,markdown)) do fx[#fx+1]=effect end end
    end
    return {db=db,fx=fx}
  end)
  misa.reg_event("transcript/response-interrupted",function(db,event,cofx)
    local owner=response(db,event.response_id); if not owner then return {db=db} end; local _,now=clock(cofx); owner.status="interrupted"; owner.completed_monotonic_ms=now; owner.elapsed_ms=math.max(0,now-owner.started_monotonic_ms)
    for i=owner.block_start,owner.block_start+owner.block_count-1 do local block=db.messages.blocks[i]; finish_block(block,nil,policy); block.interrupted=true end; return {db=db}
  end)

  local function standalone(db,kind,text,event,cofx)
    db.messages.next_id=db.messages.next_id+1; local id="transcript-"..db.messages.next_id; local owner=append_response(db,id,kind=="user" and "user" or "system",cofx,"complete")
    local model=append_block(db,owner,{id=id.."/1",kind=kind,text=copy_structural(tostring(text or ""),policy,0),streaming=false,is_error=event and event.is_error==true,level=event and event.level})
    return model
  end
  misa.reg_event("transcript/user",function(db,event,cofx) local model=standalone(db,"user",event.text,event,cofx); return {db=db,fx=noninteractive_commit(db,"transcript.user",model,cofx,markdown)} end)
  misa.reg_event("transcript/tool-result",function(db,event,cofx) standalone(db,"tool_result",event.text,event,cofx); return {db=db} end)
  misa.reg_event("transcript/harness",function(db,event,cofx) local model=standalone(db,"harness",event.text,event,cofx); return {db=db,fx=noninteractive_commit(db,"transcript.harness",model,cofx,markdown)} end)
  misa.reg_event("transcript/tool-call",function(db,event,cofx) local model=standalone(db,"tool_call","",event,cofx); model.call_id=event.id; model.name=printable_text(tostring(event.name or "tool")); model.arguments=copy_structural(event.arguments or event.arguments_json or {},policy,0); return {db=db} end)
  -- Compatibility completion input for custom agents. It is normalized once
  -- into the same response/block lifecycle rather than maintained as a shadow.
  misa.reg_event("transcript/assistant",function(db,event,cofx)
    local id=event.request_id or ("legacy-"..tostring(db.messages.next_id+1)); local owner=append_response(db,id,"assistant",cofx,"complete")
    local output={}; for index,source in ipairs(event.content or {}) do local kind=source.type=="text" and "assistant" or source.type
      local block={id=id.."/"..index,kind=kind,streaming=false,text=source.text and copy_structural(source.text,policy,0),call_id=source.id,name=source.name,arguments=source.arguments and copy_structural(source.arguments,policy,0)}; append_block(db,owner,block); if kind=="assistant" then output[#output+1]=block.text end end
    local fx={}; if #output>0 then for _,effect in ipairs(noninteractive_commit(db,"transcript.assistant",{text=table.concat(output,""),timestamp=timestamp(owner.started_wall_ms)},cofx,markdown)) do fx[#fx+1]=effect end end; return {db=db,fx=fx}
  end)
  misa.reg_event("transcript/interrupted",function(db,event,cofx)
    local result={db=db}; if not response(db,event.request_id) then local owner=append_response(db,event.request_id,"assistant",cofx,"streaming"); for index,source in ipairs(event.content or {}) do append_block(db,owner,{id=event.request_id.."/"..index,kind=source.type=="text" and "assistant" or source.type,text=source.text,streaming=false}) end end
    local owner=response(db,event.request_id); owner.status="interrupted"; for i=owner.block_start,owner.block_start+owner.block_count-1 do db.messages.blocks[i].interrupted=true end; return result
  end)
end }
