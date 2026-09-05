-- Projection adapter for overlay choice sessions.
return {setup=function()
  assert(misa.layout and misa.choice_picker_layout,"picker_view requires layout and choices")
  misa.reg_view_layer("picker",function(db,cofx)
    local picker=db.picker; if not picker then return end
    local session=picker.session
    local terminal={columns=cofx.terminal.columns,lines=cofx.terminal.lines,available_lines=cofx.available_lines}
    local geometry=misa.choice_picker_layout(session,db,terminal)
    return misa.render_component(db,"picker",{title=session.title,query=session.query,columns=geometry.columns,
      cycle_hint=misa.choice_hint("cycle"),replace_hint=misa.choice_hint("replace_view"),
      favorite_hint=session.preference_scope and misa.choice_hint("favorite") or nil},
      {columns=cofx.terminal.columns,available_lines=geometry.available_lines})
  end)
end}
