-- One-shot agent state machine. Providers are selected by effect type.
return {
  setup = function(context)
    local config = type(context.config) == "table" and context.config.agent or nil
    assert(type(config) == "table", "config.agent must be an object")
    assert(type(config.provider) == "string" and config.provider ~= "", "config.agent.provider must be a nonempty string")

    misa.reg_event("app/start", function(_, _, cofx)
      if #cofx.argv == 0 then return end
      return { fx = { { type = "dispatch", event = { type = "agent/submit", prompt = table.concat(cofx.argv, " ") } } } }
    end)

    misa.reg_event("agent/submit", function(db, event)
      assert(type(event.prompt) == "string" and event.prompt ~= "", "agent prompt must be nonempty")
      local agent = db.agent or { request_seq = 0 }
      agent.request_seq = agent.request_seq + 1
      local id = "agent-" .. tostring(agent.request_seq)
      agent.prompt, agent.response, agent.error, agent.status = event.prompt, nil, nil, "working"
      agent.active_request_id, agent.accepted_request_id = id, nil
      db.agent = agent
      local request = { type = "provider." .. config.provider, prompt = event.prompt, id = id }
      if config.system_prompt ~= nil then assert(type(config.system_prompt) == "string"); request.system_prompt = config.system_prompt end
      return { db = db, fx = { request } }
    end)

    misa.reg_event("agent/result", function(db, event)
      assert(type(event.id) == "string" and event.id ~= "", "agent result id must be nonempty")
      assert(type(event.text) == "string", "agent result text must be a string")
      local agent = db.agent
      if not agent or agent.status ~= "working" or event.id ~= agent.active_request_id then return end
      agent.response, agent.error, agent.status = event.text, nil, "done"
      agent.accepted_request_id, agent.active_request_id = event.id, nil
      return { db = db }
    end)

    misa.reg_event("agent/error", function(db, event)
      assert(type(event.id) == "string" and event.id ~= "", "agent error id must be nonempty")
      local agent = db.agent
      if not agent or agent.status ~= "working" or event.id ~= agent.active_request_id then return end
      agent.error, agent.status = tostring(event.message or "provider failed"), "error"
      agent.accepted_request_id, agent.active_request_id = event.id, nil
      return { db = db }
    end)
  end,
}
