-- Configurable semantic keybindings. Escape-prefixed printable input is exposed
-- as Alt+key without asking feature extensions to parse terminal byte streams.
local function key_of(event)
  if event.kind == "key" then return event.key end
  if event.kind == "alt" and type(event.text) == "string" then return "alt+" .. event.text:lower() end
  return event.kind
end

return {
  setup = function(context)
    local configured = type(context.config) == "table" and context.config.keybindings or nil
    configured = type(configured) == "table" and configured or {}

    misa.keybinding_action = function(context_name, event)
      local key = key_of(event)
      for _, binding in ipairs(misa.keybindings()) do
        if binding.context == context_name then
          local section = type(configured[context_name]) == "table" and configured[context_name] or {}
          local keys = section[binding.action]
          if keys == nil then keys = binding.default end
          if type(keys) == "string" then keys = { keys } end
          assert(type(keys) == "table", "configured keybinding must be a string or array")
          for _, candidate in ipairs(keys) do if candidate == key then return binding.action end end
        end
      end
    end

    misa.keybinding_hint = function(context_name, action)
      local section = type(configured[context_name]) == "table" and configured[context_name] or {}
      local keys = section[action]
      if keys == nil then
        for _, binding in ipairs(misa.keybindings()) do
          if binding.context == context_name and binding.action == action then keys = binding.default; break end
        end
      end
      if type(keys) == "string" then return keys end
      return type(keys) == "table" and keys[1] or nil
    end

    local symbols={alt="⌥",ctrl="⌃",shift="⇧",enter="↵",tab="⇥",escape="esc",arrow_up="↑",arrow_down="↓",arrow_left="←",arrow_right="→",space="space"}
    local modifiers={alt=true,ctrl=true,shift=true}
    misa.keybinding_tokens=function(key)
      local result={}; for part in tostring(key or ""):gmatch("[^+]+") do
        local text=symbols[part] or part
        if #text==1 and text:match("%l") then text=text:upper() end
        result[#result+1]={kind=modifiers[part] and "modifier" or "key",text=text}
      end; return result
    end
    misa.keybinding_text=function(key)
      local parts={}; for _,token in ipairs(misa.keybinding_tokens(key)) do parts[#parts+1]=token.text end; return table.concat(parts)
    end
    misa.render_keybinding=function(key)
      local spans={}; for _,token in ipairs(misa.keybinding_tokens(key)) do spans[#spans+1]={text=token.text,style="keybinding",token=token.kind} end; return spans
    end
    misa.render_keybinding_reference=function(entries)
      local spans={}; for index,entry in ipairs(entries or {}) do if index>1 then spans[#spans+1]={text="   ",style="plain"} end; for _,span in ipairs(misa.render_keybinding(entry.key)) do spans[#spans+1]=span end; spans[#spans+1]={text=" "..entry.label,style="label"} end; return spans
    end

    -- The native decoder already distinguishes standalone Escape from Alt
    -- chords. Normalizing only completed Alt events avoids swallowing Escape
    -- while waiting for a byte that may never arrive.
    misa.reg_interceptor({
      id = "keybindings/normalize-alt",
      before = function(tx)
        if tx.event.type == "terminal/input" and tx.event.kind == "alt" and type(tx.event.text) == "string" then
          tx.event = { type = "terminal/input", kind = "key", key = "alt+" .. tx.event.text:lower() }
        end
        return tx
      end,
    })
  end,
}
