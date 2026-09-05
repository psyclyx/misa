-- Replaceable projection for generic dialog state.
return {setup=function()
  assert(misa.dialogs,"dialog_view requires dialogs")
  misa.reg_view_layer("dialog",function(db,cofx)
    local state=db.dialog; if not state then return end
    return misa.render_component(db,"dialog",state,{columns=cofx.terminal.columns,available_lines=cofx.available_lines or cofx.terminal.lines})
  end)
end}
