-- Default editor visuals.
local function span(text,style) return {text=text,style=style} end
return {setup=function()
  assert(misa.layout and misa.layout.wrap_input,"component.editor requires wrapped input layout")
  misa.reg_component("default.editor.input",{render=function(model,context)
    return misa.layout.wrap_input(model.text,context.columns or 80,model.cursor or #(model.text or ""),"> ","user","accent")
  end})
  misa.reg_component("default.editor.completions",{render=function(model)
    local rendered={}; for _,row in ipairs(model.rows or {}) do
      local style=row.selected and "choice.row.selected" or (row.active and "choice.row.active" or "choice.row")
      local spans={span((row.marker or " ").." ",style)}
      if row.hotkey then for _,key_span in ipairs(misa.render_keybinding and misa.render_keybinding(row.hotkey) or {span(row.hotkey,"keybinding")}) do spans[#spans+1]=key_span end; spans[#spans+1]=span(" ","plain") end
      spans[#spans+1]=span(row.label,style); spans[#spans+1]=span(row.description and row.description~="" and ("  "..row.description) or "","choice.hint")
      rendered[#rendered+1]={spans=spans}
    end; return {lines=rendered}
  end})
end}
