-- Command provider translates policy effects to a direct-argv native process.
return {
  setup = function(context)
    local providers = type(context.config) == "table" and context.config.providers or nil
    local command = type(providers) == "table" and providers.command or nil
    local configured = type(command) == "table" and command.argv or nil
    assert(type(configured) == "table" and #configured > 0, "config.providers.command.argv must be a nonempty array")
    local argv = {}
    for i = 1, #configured do
      assert(type(configured[i]) == "string" and configured[i] ~= "", "command argv must contain nonempty strings")
      assert(not configured[i]:find("\0", 1, true), "command argv must not contain NUL")
      argv[i] = configured[i]
    end
    misa.reg_fx("provider.command", function(effect)
      assert(type(effect.prompt) == "string" and effect.prompt ~= "", "command prompt must be a nonempty string")
      assert(type(effect.id) == "string" and effect.id ~= "", "command id must be a nonempty string")
      local direct = {}; for i = 1, #argv do direct[i] = argv[i] end
      direct[#direct + 1] = effect.prompt
      return { type = "process/run", argv = direct, id = effect.id, completion = "provider/command-complete" }
    end)
    misa.reg_event("provider/command-complete", function(db, event)
      assert(type(event.id) == "string" and event.id ~= "", "command completion id must be nonempty")
      local next_event
      if event.ok then next_event = { type = "agent/result", id = event.id, text = event.stdout }
      else next_event = { type = "agent/error", id = event.id, message = event.stderr ~= "" and event.stderr or ("command exited " .. tostring(event.status)) } end
      return { fx = { { type = "dispatch", event = next_event } } }
    end)
  end,
}
