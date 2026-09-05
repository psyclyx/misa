-- Default editor visuals.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  assert(misa.layout and misa.layout.wrap_input,"component.editor requires wrapped input layout")
  misa.reg_component("default.editor.input",{render=function(model,context)
    return misa.layout.wrap_input(model.text,context.columns or 80,model.cursor or #(model.text or ""),"> ","user","accent")
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
