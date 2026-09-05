-- Registry and projection for small semantic status values. Features declare
-- values; configuration chooses presentation and order. No feature knows where
-- or how the resulting row is composed.
return { setup = function(context)
  local definitions, sealed = {}, false
  local root = type(context.config) == "table" and context.config.status or nil
  root = type(root) == "table" and root or {}
  local configured = root.indicators or {
    { id="activity", representation="icon", priority=100 },
    { id="model", representation="label", priority=90 },
    { id="effort", representation="label", hotkey=true, priority=70 },
    { id="session", representation="icon", priority=40 },
    { id="context", representation="label", priority=80 },
    { id="transcript-detail", representation="label", hotkey=true, priority=20 },
  }
  assert(type(configured) == "table", "config.status.indicators must be an array")

  misa.reg_indicator = function(definition)
    assert(not sealed, "indicator registrations are sealed")
    assert(type(definition) == "table" and type(definition.id) == "string" and definition.id ~= "", "indicator ID must be nonempty")
    assert(type(definition.value) == "function", "indicator must provide a value projection")
    assert(definition.label == nil or type(definition.label) == "string", "indicator label must be a string")
    assert(definition.icon == nil or type(definition.icon) == "string", "indicator icon must be a string")
    assert(definition.hotkey == nil or (type(definition.hotkey) == "table" and type(definition.hotkey.context) == "string" and type(definition.hotkey.action) == "string"), "indicator hotkey must name a context and action")
    assert(definitions[definition.id] == nil, "duplicate indicator: " .. definition.id)
    definitions[definition.id] = definition
  end

  misa.reg_interceptor({ id="indicators/seal", before=function(tx)
    if tx.event.type == "app/start" then sealed = true end
    return tx
  end })

  misa.indicators_projection = function(db, render_context)
    local values = {}
    for index, selection in ipairs(configured) do
      if type(selection) == "string" then selection = { id=selection } end
      assert(type(selection) == "table" and type(selection.id) == "string", "invalid status indicator selection")
      local definition = definitions[selection.id]
      if definition then
        local value = definition.value(db)
        if value ~= nil and value ~= false and tostring(value) ~= "" then
          local representation = selection.representation or "label"
          assert(representation == "label" or representation == "icon", "indicator representation must be label or icon")
          local label = representation == "icon" and (definition.icon or definition.label or selection.id) or (definition.label or selection.id)
          local hotkey
          if selection.hotkey == true and definition.hotkey and misa.keybinding_hint then
            hotkey = misa.keybinding_hint(definition.hotkey.context, definition.hotkey.action)
          elseif type(selection.hotkey) == "string" then hotkey = selection.hotkey end
          values[#values + 1] = {
            id=selection.id, label=label, value=tostring(value), hotkey=hotkey,
            priority=tonumber(selection.priority) or (1000-index),
          }
        end
      end
    end
    local context_copy = {}; for key, value in pairs(render_context or {}) do context_copy[key] = value end
    return misa.render_component(db, "status.indicators", { indicators=values }, context_copy).lines
  end
end }
