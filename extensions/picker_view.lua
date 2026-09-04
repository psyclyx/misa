-- Default picker projection. Picker state/input remain in picker.lua.
local function first_index(panel, room)
  if room <= 0 or #panel.filtered <= room then return 1 end
  return math.max(1, math.min(panel.highlight - math.floor(room / 2), #panel.filtered - room + 1))
end
return { setup = function()
  assert(misa.layout, "picker_view requires layout")
  misa.reg_view_layer("picker", function(db, cofx)
    local state = db.picker; if not state then return end
    local count = #misa.layout.columns(cofx.terminal.columns, 28, math.min(#state.panels, 3), 2)
    local choice_room = math.max(0, math.min(9, (cofx.available_lines or cofx.terminal.lines) - 3))
    local panel_hint = misa.keybinding_hint and misa.keybinding_hint("picker", "next_panel") or "tab"
    local favorite = ""; if state.preference_scope then
      local hint = misa.keybinding_hint and misa.keybinding_hint("picker", "favorite") or "alt+v"; favorite = hint and ("    " .. hint .. " favorite") or ""
    end
    local hints = {}; for visible=1,count do hints[visible]={}; for slot=1,choice_room do
      local hint=misa.keybinding_hint and misa.keybinding_hint("picker","option_"..visible.."_"..slot) or nil
      if not misa.keybinding_hint then local defaults={{"1","2","3","4","5","6","7","8","9"},{"q","w","e","r","t","y","u","i","o"},{"a","s","d","f","g","h","j","k","l"}}; hint="alt+"..defaults[visible][slot] end
      hints[visible][slot]=hint
    end end
    return misa.render_component(db, "picker", {
      state=state, visible_panels=count, choice_room=choice_room, first_index=first_index,
      panel_hint=panel_hint or "", favorite_hint=favorite, option_hints=hints,
    }, {columns=cofx.terminal.columns,available_lines=cofx.available_lines or cofx.terminal.lines})
  end)
end }
