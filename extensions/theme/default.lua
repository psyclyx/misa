return { setup = function()
  misa.reg_theme("default", {
    palette = {
      text = "default", muted = "bright_black", accent = "cyan",
      user = "green", assistant = "blue", thinking = "magenta",
      tool = "yellow", error = "bright_red",
    },
    styles = {
      plain = { foreground = "text" },
      dim = { dim = true },
      bold = { bold = true }, italic = { italic = true },
      strikethrough = { strikethrough = true }, underline = { underline = true },
      accent = { foreground = "accent" }, code = { foreground = "accent" },
      link = { foreground = "accent", underline = true }, quote = { dim = true },

      user = { foreground = "user" }, assistant = { foreground = "assistant" },
      thinking = { foreground = "thinking", dim = true }, tool = { foreground = "tool" },
      error = { foreground = "error", bold = true },
      ["rail.user"] = { foreground = "user" }, ["rail.assistant"] = { foreground = "assistant" },
      ["rail.thinking"] = { foreground = "thinking" }, ["rail.tool"] = { foreground = "tool" },
      ["rail.error"] = { foreground = "error", bold = true }, ["rail.harness"] = { foreground = "muted" },

      label = { foreground = "text", dim = true }, value = { foreground = "text" },
      keybinding = { foreground = "accent", dim = true },
      ["choice.prompt"] = { foreground = "accent" }, ["choice.query"] = { foreground = "text" },
      ["choice.hint"] = { foreground = "text", dim = true }, ["choice.view"] = { bold = true },
      ["choice.view.active"] = { foreground = "accent", bold = true }, ["choice.row"] = { foreground = "text" },
      ["choice.row.active"] = { foreground = "accent" }, ["choice.row.selected"] = { bold = true },
      ["choice.empty"] = { foreground = "text", dim = true },
      ["dialog.title"] = { foreground = "accent", bold = true }, ["dialog.message"] = { foreground = "text" },
      ["dialog.label"] = { foreground = "text", dim = true }, ["dialog.value"] = { foreground = "text" },
      ["dialog.code"] = { foreground = "accent", bold = true }, ["dialog.progress"] = { foreground = "text", dim = true },
      ["dialog.input"] = { foreground = "text" }, ["dialog.hint"] = { foreground = "text", dim = true },
    },
  })
end }
