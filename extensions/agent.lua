-- Agent state machine. Providers are selected by effect type; no direct calls or output.
return {
  setup = function(context)
    local config = type(context.config) == "table" and context.config.agent or nil
    assert(type(config) == "table", "config.agent must be an object")
    assert(type(config.provider) == "string" and config.provider ~= "", "config.agent.provider must be a nonempty string")

    misa.reg_event("app/start", function(db, _, cofx)
      if #cofx.argv == 0 then return end
      return { fx = { { type = "dispatch", event = { type = "agent/submit", prompt = table.concat(cofx.argv, " ") } } } }
    end)
    misa.reg_event("agent/submit", function(db, event)
      assert(type(event.prompt) == "string" and event.prompt ~= "", "agent prompt must be nonempty")
      db.agent_request_seq = (db.agent_request_seq or 0) + 1
      local id = "agent-" .. tostring(db.agent_request_seq)
      db.prompt, db.response, db.error, db.status = event.prompt, nil, nil, "working"
      db.active_request_id, db.accepted_request_id = id, nil
      local request = { type = "provider." .. config.provider, prompt = event.prompt, id = id }
      if config.system_prompt ~= nil then assert(type(config.system_prompt) == "string"); request.system_prompt = config.system_prompt end
      return { db = db, fx = { request } }
    end)
    misa.reg_event("agent/result", function(db, event)
      assert(type(event.id) == "string" and event.id ~= "", "agent result id must be nonempty")
      assert(type(event.text) == "string", "agent result text must be a string")
      if db.status ~= "working" or event.id ~= db.active_request_id then return end
      db.response, db.error, db.status = event.text, nil, "done"
      db.accepted_request_id, db.active_request_id = event.id, nil
      return { db = db }
    end)
    misa.reg_event("agent/error", function(db, event)
      assert(type(event.id) == "string" and event.id ~= "", "agent error id must be nonempty")
      if db.status ~= "working" or event.id ~= db.active_request_id then return end
      db.error, db.status = tostring(event.message or "provider failed"), "error"
      db.accepted_request_id, db.active_request_id = event.id, nil
      return { db = db }
    end)
  end,
}
