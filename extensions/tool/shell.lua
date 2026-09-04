-- Shell tool policy over Zig's direct process capability.
return {
  setup = function(context)
    local config = type(context.config) == "table" and context.config.tools or nil
    config = type(config) == "table" and config.shell or nil
    config = type(config) == "table" and config or {}
    local executable = config.executable or "sh"
    assert(type(executable) == "string" and executable ~= "", "config.tools.shell.executable must be nonempty")

    misa.reg_tool({
      name = "shell",
      description = "Run a shell command in Misa's working directory and return its captured output.",
      input_schema = {
        type = "object",
        properties = { command = { type = "string", description = "Shell command to execute" } },
        required = { "command" }, additionalProperties = false,
      },
      effect = "tool.shell/run",
    })

    misa.reg_fx("tool.shell/run", function(effect)
      local arguments = assert(type(effect.arguments) == "table" and effect.arguments, "shell arguments must be an object")
      assert(type(arguments.command) == "string" and arguments.command ~= "", "command must be nonempty")
      return {
        type = "process/run", argv = { executable, "-lc", arguments.command },
        completion = "tool/shell-complete", id = effect.tool_call_id,
      }
    end)

    misa.reg_event("tool/shell-complete", function(_, event)
      local text = event.stdout or ""
      if event.stderr and event.stderr ~= "" then
        if text ~= "" and text:sub(-1) ~= "\n" then text = text .. "\n" end
        text = text .. event.stderr
      end
      if text == "" then text = event.ok and "command completed with no output" or ("command exited " .. tostring(event.status)) end
      return { fx = { { type = "dispatch", event = {
        type = "tool/result", tool_call_id = event.id, text = text, is_error = not event.ok,
      } } } }
    end)
  end,
}
