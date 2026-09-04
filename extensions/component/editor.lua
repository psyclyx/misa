-- Default editor visuals.
local function span(text,style) return {text=text,style=style} end
local function lines(text)
  local result={}; text=tostring(text or ""):gsub("\r\n","\n"):gsub("\r","\n")
  for line in (text.."\n"):gmatch("(.-)\n") do result[#result+1]={spans={span(line,"user")}} end; return result
end
return {setup=function()
  misa.reg_component("default.editor.input",{render=function(model)
    local rendered=lines(model.text); table.insert(rendered[1].spans,1,span("> ","accent")); return {lines=rendered}
  end})
  misa.reg_component("default.editor.completions",{render=function(model)
    local rendered={}; for index,candidate in ipairs(model.matches) do local active=model.active==index; rendered[#rendered+1]={spans={
      span(active and "> " or "  ",active and "accent" or "plain"),span(candidate.label,"bold"),span("  "..candidate.description,"dim"),
    }} end; return {lines=rendered}
  end})
end}
