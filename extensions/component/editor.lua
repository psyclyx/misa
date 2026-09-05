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
    local rendered={}; for _,row in ipairs(model.rows or {}) do rendered[#rendered+1]={spans={
      span((row.marker or " ").." ",row.active and "choice.row.active" or "choice.row"),
      span(row.hotkey and (row.hotkey.." ") or "", "choice.hint"),
      span(row.label,row.active and "choice.row.active" or (row.selected and "choice.row.selected" or "choice.row")),
      span(row.description and row.description~="" and ("  "..row.description) or "","choice.hint"),
    }} end; return {lines=rendered}
  end})
end}
