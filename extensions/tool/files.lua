-- File tools are Lua policy over small native file effects. Paths are used as
-- supplied; filesystem sandboxing belongs to the environment running Misa.
local function schema(properties, required)
  return { type = "object", properties = properties, required = required, additionalProperties = false }
end

return {
  setup = function()
    misa.reg_tool({
      name = "read_file",
      description = "Read a UTF-8 text file. Paths may be absolute or relative to Misa's working directory.",
      input_schema = schema({ path = { type = "string", description = "File path" } }, { "path" }),
      effect = "tool.files/read",
    })
    misa.reg_tool({
      name = "write_file",
      description = "Create or replace a UTF-8 text file with the exact supplied content.",
      input_schema = schema({
        path = { type = "string", description = "File path" },
        content = { type = "string", description = "Complete new file content" },
      }, { "path", "content" }),
      effect = "tool.files/write",
    })
    misa.reg_tool({
      name = "edit_file",
      description = "Replace one exact, uniquely occurring string in a UTF-8 text file.",
      input_schema = schema({
        path = { type = "string", description = "File path" },
        old_text = { type = "string", description = "Exact text to replace; it must occur once" },
        new_text = { type = "string", description = "Replacement text" },
      }, { "path", "old_text", "new_text" }),
      effect = "tool.files/edit",
    })

    local function argument(effect, name)
      local arguments = assert(type(effect.arguments) == "table" and effect.arguments, "file tool arguments must be an object")
      local value = arguments[name]
      assert(type(value) == "string", name .. " must be a string")
      return value
    end

    misa.reg_fx("tool.files/read", function(effect)
      return {
        type = "file/read", path = argument(effect, "path"),
        completion = "tool/files-complete", id = effect.tool_call_id,
      }
    end)
    misa.reg_fx("tool.files/write", function(effect)
      return {
        type = "file/write", path = argument(effect, "path"), content = argument(effect, "content"),
        completion = "tool/files-complete", id = effect.tool_call_id,
      }
    end)
    misa.reg_fx("tool.files/edit", function(effect)
      return {
        type = "file/edit", path = argument(effect, "path"), content = argument(effect, "old_text"),
        replacement = argument(effect, "new_text"), completion = "tool/files-complete", id = effect.tool_call_id,
      }
    end)

    misa.reg_event("tool/files-complete", function(_, event)
      return { fx = { { type = "dispatch", event = {
        type = "tool/result", tool_call_id = event.id,
        text = event.ok and event.text or event.message, is_error = not event.ok,
      } } } }
    end)
  end,
}
