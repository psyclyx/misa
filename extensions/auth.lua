-- Authentication commands; secret handling and provider-specific flows remain native.
return {
  setup = function()
    local providers = misa.auth_providers()
    local function provider_by_id(id) for _,provider in ipairs(providers) do if provider.id==id then return provider end end end
    local function auth_effect(action,provider,id)
      return {type="auth/command",action=action,provider=provider.id,strategy=provider.strategy,profile=provider.profile,
        completion="auth/complete",interaction="auth/interaction",id=id}
    end
    local function model_provider_for(id)
      for _, provider in ipairs(providers) do if provider.id == id then return provider.model_provider end end
    end

    misa.reg_interceptor({
      id = "auth/startup-state",
      before = function(tx)
        if tx.event.type == "app/start" and not tx.db.auth_startup then
          tx.db.auth_startup = { pending_status = #providers, pending_discovery = 0, ready = #providers == 0 }
        end
        return tx
      end,
    })
    misa.reg_event("app/start", function(db)
      local effects = {}
      for _, provider in ipairs(providers) do
        local effect=auth_effect("status",provider,provider.model_provider); effect.completion="auth/provider-status"
        effects[#effects + 1] = effect
      end
      if #providers == 0 then effects[#effects + 1] = { type = "dispatch", event = { type = "auth/startup-ready" } } end
      return { db = db, fx = effects }
    end)
    misa.reg_event("auth/provider-status", function(db, event)
      local startup = assert(db.auth_startup, "auth startup state is missing")
      startup.pending_status = startup.pending_status - 1
      local available = event.ok and event.logged_in == true
      local effects = { { type = "dispatch", event = {
        type = "models/provider-availability", provider = event.id, available = available,
        subscription_type = event.subscription_type,
      } } }
      local declaration
      for _, provider in ipairs(providers) do if provider.model_provider == event.id then declaration = provider; break end end
      if available and declaration and declaration.discover_models then
        startup.pending_discovery = startup.pending_discovery + 1
        effects[#effects + 1] = { type = "dispatch", event = { type = "models/discover", provider = event.id } }
      end
      if startup.pending_status == 0 and startup.pending_discovery == 0 then
        startup.ready = true
        effects[#effects + 1] = { type = "dispatch", event = { type = "auth/startup-ready" } }
      end
      return { db = db, fx = effects }
    end)
    misa.reg_event("models/discovery-complete", function(db)
      local startup = db.auth_startup
      if not startup or startup.ready then return end
      startup.pending_discovery = math.max(0, startup.pending_discovery - 1)
      if startup.pending_status == 0 and startup.pending_discovery == 0 then
        startup.ready = true
        return { db = db, fx = { { type = "dispatch", event = { type = "auth/startup-ready" } } } }
      end
      return { db = db }
    end)

    for _, command in ipairs({
      { name = "/login", description = "Log in to a provider", action = "login" },
      { name = "/logout", description = "Log out of a provider", action = "logout" },
      { name = "/status", description = "Show provider login state", action = "status" },
    }) do
      local item = command
      local event_type = "auth/" .. item.action
      misa.reg_command({ name = item.name, description = item.description, event = event_type, completion = "auth-provider", choice_purpose = "auth" })
      misa.reg_event(event_type, function(_, event, cofx)
        local provider = type(event.arguments) == "string" and event.arguments:match("^%s*(%S+)%s*$") or nil
        if not provider then
          return { fx = { { type = "dispatch", event = {
            type = "auth/complete", id = item.action, ok = false,
            message = "usage: " .. item.name .. " <provider>",
          } } } }
        end
        local effects = {}
        if cofx.terminal.interactive then effects[#effects + 1] = { type = "dispatch", event = {
          type = "transcript/harness", text = item.action .. " " .. provider .. "…", level = "info",
        } } end
        local declaration=provider_by_id(provider)
        if not declaration then effects[#effects+1]={type="dispatch",event={type="auth/complete",id=item.action..":"..provider,provider=provider,ok=false,message="unknown provider"}}
        elseif item.action=="login" and (declaration.strategy=="api_key" or declaration.strategy=="cli_handoff") then
          effects[#effects+1]={type="dispatch",event={type="dialog/open",id="auth-handoff",correlation=item.action..":"..provider,completion="auth/handoff-confirmed",kind="alert",title="Terminal handoff",message="This provider requires temporary terminal input. Continue to leave the managed screen, then return automatically.",cancellable=true,actions={{id="continue",label="enter continue",primary=true}},hints={"secret input is handled natively"}}}
        else effects[#effects+1]=auth_effect(item.action,declaration,item.action..":"..provider) end
        return { fx = effects }
      end)
    end

    misa.reg_event("auth/handoff-confirmed",function(db,event)
      if event.cancelled then return {db=db,fx={{type="terminal/read"}}} end
      local action,provider=event.correlation:match("^([^:]+):(.+)$"); local declaration=provider_by_id(provider)
      return {db=db,fx={auth_effect(action,assert(declaration),event.correlation)}}
    end)
    misa.reg_event("auth/interaction",function(db,event)
      local dialog={type=db.dialog and "dialog/update" or "dialog/open",id=event.id,correlation=event.correlation or event.id,
        completion="auth/dialog-action",kind=event.kind,title=event.title,message=event.message,url=event.url,code=event.code,
        progress=event.progress,cancellable=event.cancellable,input=event.input,actions=event.actions,hints=event.hints}
      return {db=db,fx={{type="dispatch",event=dialog},{type="terminal/read"}}}
    end)
    misa.reg_event("auth/dialog-action",function(db,event)
      if event.cancelled then return {db=db,fx={{type="operation/cancel",id=event.id},{type="terminal/read"}}} end
      return {db=db,fx={{type="auth/respond",id=event.id,correlation=event.correlation,action=event.action,value=event.value}}}
    end)
    misa.reg_event("auth/complete", function(db, event)
      if db.dialog and db.dialog.id==event.id then db.dialog=nil end
      local message = event.message
      if event.subscription_type and event.subscription_type ~= misa.json_null then message = message .. " (" .. event.subscription_type .. ")" end
      local effects = { { type = "dispatch", event = {
        type = "transcript/harness", text = message, level = event.ok and "info" or "error",
      } } }
      local provider = model_provider_for(event.provider)
      if provider and event.ok then
        effects[#effects + 1] = { type = "dispatch", event = {
          type = "models/provider-availability", provider = provider, available = event.logged_in == true,
          subscription_type = event.subscription_type,
        } }
        if event.logged_in == true then effects[#effects + 1] = { type = "dispatch", event = { type = "models/discover", provider = provider } } end
      end
      effects[#effects + 1] = { type = "dispatch", event = { type = "auth/ready" } }
      return { fx = effects }
    end)
    misa.reg_event("auth/ready", function(db)
      return { db = db, fx = { { type = "terminal/read" } } }
    end)
  end,
}
