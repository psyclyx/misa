-- A deliberately small test extension. It demonstrates the lifecycle and
-- context without supplying any coding-agent policy.
return {
  setup = function(context)
    context.misa.register("inspect.message", function()
      return context.config_json
    end)
  end,

  run = function(context)
    local messages = context.misa.call("inspect.message")
    io.write("misa default config\n")
    io.write("config: " .. messages[1] .. "\n")
    io.write("argv: " .. table.concat(context.argv, ", ") .. "\n")
  end,
}
