-- Authentication commands; secret handling and provider-specific flows remain native.
return {
  setup = function()
    for _, command in ipairs({
      { name = "/login", description = "Log in to a provider: /login <provider>", action = "login" },
      { name = "/logout", description = "Log out of a provider: /logout <provider>", action = "logout" },
      { name = "/status", description = "Show provider login state: /status <provider>", action = "status" },
    }) do
      local item = command
      local event_type = "auth/" .. item.action
      misa.reg_command({ name = item.name, description = item.description, event = event_type, completion = "auth-provider" })
      misa.reg_event(event_type, function(_, event)
        local provider = type(event.arguments) == "string" and event.arguments:match("^%s*(%S+)%s*$") or nil
        if not provider then
          return { fx = { { type = "dispatch", event = {
            type = "auth/complete", id = item.action, ok = false,
            message = "usage: " .. item.name .. " <provider>",
          } } } }
        end
        return { fx = { {
          type = "auth/command", action = item.action, provider = provider,
          completion = "auth/complete", id = item.action .. ":" .. provider,
        } } }
      end)
    end

    misa.reg_event("auth/complete", function(_, event)
      local style = event.ok and "plain" or "error"
      return { fx = {
        { type = "view/commit", lines = { { spans = { { text = event.message, style = style } } } } },
        { type = "dispatch", event = { type = "auth/ready" } },
      } }
    end)
    misa.reg_event("auth/ready", function(db)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)
  end,
}
