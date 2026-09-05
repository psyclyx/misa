return { setup = function()
  misa.reg_theme("default", {
    plain = "plain", dim = "dim", bold = "bold", accent = "accent",
    user = "user", assistant = "assistant", thinking = "dim", tool = "dim", error = "error",
    ["choice.prompt"] = "accent", ["choice.query"] = "plain", ["choice.hint"] = "dim",
    ["choice.view"] = "bold", ["choice.view.active"] = "accent", ["choice.row"] = "plain",
    ["choice.row.active"] = "accent", ["choice.row.selected"] = "bold", ["choice.empty"] = "dim",
    ["indicator.label"] = "dim", ["indicator.value"] = "plain", ["indicator.hotkey"] = "dim",
  })
end }
