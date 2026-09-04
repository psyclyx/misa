-- Default status visual.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  misa.reg_component("default.status.metrics",{render=function(model)
    local spans={}; for index,metric in ipairs(model.metrics) do
      if index>1 then spans[#spans+1]=span("  ","plain") end
      spans[#spans+1]=span(metric.prefix.." ","dim"); spans[#spans+1]=span(metric.value,metric.style or "plain")
    end
    return {lines=#spans>0 and {{spans=spans}} or {}}
  end})
end}
