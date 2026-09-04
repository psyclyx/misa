-- Provider backed by a trusted local executable. All values are shell-quoted.
local function quote(value)
  assert(type(value) == "string", "command arguments must be strings")
  return "'" .. value:gsub("'", "'\\''") .. "'"
end

return {
  setup = function(context)
    local providers = type(context.config) == "table" and context.config.providers or nil
    local command = type(providers) == "table" and providers.command or nil
    local argv = type(command) == "table" and command.argv or nil
    assert(type(argv) == "table" and #argv > 0, "config.providers.command.argv must be a nonempty array of strings")

    local configured = {}
    for index = 1, #argv do
      assert(type(argv[index]) == "string" and argv[index] ~= "", "config.providers.command.argv must contain nonempty strings")
      assert(not argv[index]:find("\0", 1, true), "config.providers.command.argv must not contain NUL")
      configured[index] = quote(argv[index])
    end

    context.misa.register("provider.command.complete", function(request)
      assert(type(request) == "table" and type(request.prompt) == "string", "command provider request.prompt must be a string")
      local words = {}
      for index = 1, #configured do words[index] = configured[index] end
      words[#words + 1] = quote(request.prompt)
      local pipe, open_error = io.popen(table.concat(words, " "), "r")
      assert(pipe, "cannot start command provider: " .. tostring(open_error))
      local text, read_error = pipe:read("*a")
      local ok, reason, status = pipe:close()
      assert(text ~= nil, "cannot read command provider output: " .. tostring(read_error))
      assert(ok == true, "command provider failed: " .. tostring(reason) .. " " .. tostring(status))
      return { text = text }
    end)
  end,
}
