-- Agent-owned facts normalized for generic indicator projections.
local function metric(value)
  if type(value) ~= "number" then return tostring(value or "?") end
  local units={{1e9,"G"},{1e6,"M"},{1e3,"k"}}
  for _,unit in ipairs(units) do if math.abs(value)>=unit[1] then local scaled=value/unit[1]; return (scaled>=10 and string.format("%.0f",scaled) or string.format("%.1f",scaled)):gsub("%.0$","")..unit[2] end end
  return tostring(value)
end
return { setup = function()
  misa.reg_event("app/start", function(db)
    db.status={mode="ready",usage={input_tokens=0,output_tokens=0},last_usage={}}; return {db=db}
  end)
  misa.reg_event("agent/status", function(db,event)
    db.status.mode=event.status; if event.usage then db.status.usage=event.usage end; if event.last_usage then db.status.last_usage=event.last_usage end; return {db=db}
  end)
  misa.reg_event("agent/usage", function(db,event)
    db.status.usage=event.usage or db.status.usage
    db.status.last_usage=event.last_usage or db.status.last_usage; return {db=db}
  end)
  if misa.reg_indicator then
    misa.reg_indicator({id="activity",label="activity",icon="●",value=function(db)
      local mode=(db.status or {}).mode or "ready"
      if mode~="ready" and misa.animation_frame then mode=mode..misa.animation_frame(db,"status") end
      return mode
    end})
    misa.reg_indicator({id="session",label="session",icon="Σ",value=function(db)
      local usage=(db.status or {}).usage or {}; return metric((usage.input_tokens or 0)+(usage.output_tokens or 0))
    end})
    misa.reg_indicator({id="context",label="ctx",icon="◫",value=function(db)
      local last=(db.status or {}).last_usage or {}; local used=(last.input_tokens or 0)+(last.output_tokens or 0)
      local model=misa.selected_model_projection and misa.selected_model_projection(db) or nil
      return model and (metric(used).."/"..metric(model.context_window)) or metric(used)
    end})
    misa.status_projection=function(db,context) return misa.indicators_projection(db,context) end
  else
    -- Small compatibility projection for profiles that intentionally omit the
    -- indicators service.
    misa.status_projection=function(db)
      if not misa.render_component then return {} end
      local state=db.status or {}; return misa.render_component(db,"status.metrics",{metrics={{prefix="●",value=state.mode or "ready"}}}).lines
    end
  end
end }
