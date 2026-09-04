-- Commands declare completion data; this extension turns an interactive empty
-- invocation into a picker transaction and resumes the original command.
local function empty(arguments)
  return type(arguments) ~= "string" or arguments:match("^%s*$") ~= nil
end

return {
  setup = function()
    misa.reg_interceptor({
      id = "command-choice/open",
      before = function(tx)
        local event = tx.event
        if event.type == "command-choice/open" or type(event.command) ~= "string" or not empty(event.arguments) then return tx end
        local command = misa.command(event.command)
        if not command or not (command.completion or command.complete) then return tx end
        tx.event = { type = "command-choice/open", command = command.name }
        return tx
      end,
    })

    misa.reg_event("command-choice/open", function(db, event)
      local command = assert(misa.command(event.command), "command choice references an unknown command")
      assert(misa.picker, "interactive command choices require the picker extension")
      db.command_choice = db.command_choice or { sequence = 0, pending = {} }
      local choices = db.command_choice
      choices.sequence = choices.sequence + 1
      local token = "command:" .. tostring(choices.sequence)
      choices.pending[token] = { event = command.event, command = command.name }
      return { db = db, fx = { { type = "dispatch", event = {
        type = "picker/open", id = "command-choice", token = token,
        title = command.name:sub(2), completion = "command-choice/selected",
        items = misa.command_completions(command, "", db),
        selected = command.selected and command.selected(db) or nil,
        preference_scope = command.preference_scope or ("command:" .. command.name),
      } } } }
    end)

    misa.reg_event("command-choice/selected", function(db, event)
      local choices = db.command_choice
      local pending = choices and choices.pending[event.picker_token] or nil
      if event.picker ~= "command-choice" or not pending then return end
      choices.pending[event.picker_token] = nil
      if event.cancelled == true then return { db = db, fx = { { type = "terminal/read" } } } end
      assert(type(event.value) == "string" and event.value ~= "", "command choice must be nonempty")
      return { db = db, fx = { { type = "dispatch", event = {
        type = pending.event, command = pending.command, arguments = event.value, resumed_choice = true,
      } } } }
    end)
  end,
}
