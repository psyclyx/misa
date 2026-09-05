-- Projection adapter: choice_layout is the sole geometry owner.
return {setup=function()
  assert(misa.choice_picker_layout,"picker_view requires choice_layout")
  misa.reg_view_layer("picker",function(db,cofx)
    local picker=db.picker; if not picker then return end
    local geometry=misa.choice_picker_layout(picker.session,db,{columns=cofx.terminal.columns,lines=cofx.terminal.lines,available_lines=cofx.available_lines})
    return misa.render_component(db,"picker",geometry,{columns=cofx.terminal.columns,available_lines=geometry.available_lines})
  end)
end}
