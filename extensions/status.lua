-- Compact application metrics projected below the editor.
local function metric(value)
  if type(value) ~= "number" then return tostring(value or "?") end
  local units = { {1e9,"G"}, {1e6,"M"}, {1e3,"k"} }
  for _, unit in ipairs(units) do if math.abs(value) >= unit[1] then
    local scaled=value/unit[1]; return (scaled>=10 and string.format("%.0f",scaled) or string.format("%.1f",scaled)):gsub("%.0$","")..unit[2]
  end end
  return tostring(value)
end
return { setup = function()
  misa.reg_event("app/start", function(db)
    db.status = { mode = "ready", usage = { input_tokens = 0, output_tokens = 0 }, last_usage = {} }; return { db = db }
  end)
  misa.reg_event("agent/status", function(db, event)
    db.status.mode = event.status; if event.usage then db.status.usage = event.usage end; if event.last_usage then db.status.last_usage = event.last_usage end; return { db = db }
  end)
  misa.reg_event("agent/usage", function(db, event)
    db.status.usage, db.status.last_usage = event.usage or db.status.usage, event.last_usage or db.status.last_usage; return { db = db }
  end)
  misa.status_projection = function(db)
    local state=db.status or {}; local usage,last=state.usage or {},state.last_usage or {}
    local model=misa.selected_model_projection and misa.selected_model_projection(db) or nil
    local messages=misa.messages_projection and misa.messages_projection(db) or {verbose=false}
    local options=misa.request_options_projection and misa.request_options_projection(db) or {values={}}
    local total=(usage.input_tokens or 0)+(usage.output_tokens or 0); local context=(last.input_tokens or 0)+(last.output_tokens or 0); local mode=state.mode or "ready"
    if mode~="ready" and misa.animation_frame then mode=mode..misa.animation_frame(db,"status") end
    local hint=misa.keybinding_hint and misa.keybinding_hint("global","toggle_verbose") or "alt+t"; local metrics={
      {prefix="●",value=mode,style=mode=="ready" and "user" or (mode=="error" and "error" or "accent")},
      {prefix="model",value=model and model.id or "none",style="bold"},
      {prefix="Σ",value=metric(total),style="bold"},
      {prefix="ctx",value=model and (metric(context).."/"..metric(model.context_window)) or metric(context),style="accent"},
      {prefix="detail",value=messages.verbose and "verbose" or ("compact "..hint),style=messages.verbose and "accent" or "dim"},
    }
    if options.values.reasoning_effort~=nil then metrics[#metrics+1]={prefix="effort",value=tostring(options.values.reasoning_effort),style="accent"} end
    return misa.render_component(db,"status.metrics",{metrics=metrics}).lines
  end
end }
