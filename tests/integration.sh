#!/bin/sh
set -eu
case "$1" in /*) MISA_BIN="$1" ;; *) MISA_BIN="$PWD/${1#./}" ;; esac
work="$(mktemp -d)"; stage=start
trap 'status=$?; if [ "$status" -ne 0 ]; then echo "integration failed during $stage" >&2; fi; rm -rf "$work"; exit "$status"' EXIT
export MISA_AUTH_FILE="$work/session-auth.json"
export MISA_STATE_FILE="$work/application-state.json"

# No extensions remains completely silent.
printf '{}' >"$work/empty.json"
[ -z "$(MISA_AUTH_FILE="$work/empty-auth.json" MISA_CONFIG="$work/empty.json" "$MISA_BIN")" ]

stage=authentication
# Explicit API-key login writes only the XDG credential store with restrictive
# permissions; secrets never enter the ordinary config.
printf 'test-secret\n' | env -u MISA_AUTH_FILE XDG_STATE_HOME="$work/state" "$MISA_BIN" login openai 2>"$work/login-output"
grep -F 'saved openai credential' "$work/login-output" >/dev/null
! grep -F 'test-secret' "$work/login-output" >/dev/null
[ "$(stat -c %a "$work/state/misa")" = 700 ]
[ "$(stat -c %a "$work/state/misa/auth.json")" = 600 ]
grep -F 'test-secret' "$work/state/misa/auth.json" >/dev/null
[ "$(env -u MISA_AUTH_FILE XDG_STATE_HOME="$work/state" "$MISA_BIN" status openai)" = 'logged in' ]
env -u MISA_AUTH_FILE XDG_STATE_HOME="$work/state" "$MISA_BIN" logout openai
[ "$(env -u MISA_AUTH_FILE XDG_STATE_HOME="$work/state" "$MISA_BIN" status openai)" = 'logged out' ]
! grep -F 'test-secret' "$work/state/misa/auth.json" >/dev/null
mkdir "$work/bin"
cat >"$work/bin/claude" <<'SH'
#!/bin/sh
[ "$1" = auth ]
case "$2" in login|logout) ;; status) printf '{"loggedIn":true,"subscriptionType":"max"}\n' ;; *) exit 1 ;; esac
SH
chmod +x "$work/bin/claude"
PATH="$work/bin:$PATH" "$MISA_BIN" login claude
[ "$(PATH="$work/bin:$PATH" "$MISA_BIN" status claude)" = 'logged in (max)' ]
PATH="$work/bin:$PATH" "$MISA_BIN" logout claude
[ ! -e "$work/session-auth.json" ]
printf '%s' '{"extensions":["auth","protocol.openai","provider.openai","fuzzy","command_choice","preferences","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","picker","picker_view","editor","ui"],"config":{"providers":{"openai":{"discover_models":false}}}}' >"$work/auth-ui.json"
[ "$(printf '/status op\t\n' | MISA_CONFIG="$work/auth-ui.json" "$MISA_BIN")" = 'logged out' ]
[ "$(printf '/status\nopenai\n' | MISA_CONFIG="$work/auth-ui.json" "$MISA_BIN")" = 'logged out' ]

stage=provider-composition
# Real provider declarations compose without credentials until they are used.
printf '%s' '{"extensions":["protocol.anthropic","provider.anthropic","provider.kimi","protocol.openai","provider.openai","provider.openrouter","provider.openai-codex","provider.claude","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"anthropic/claude-sonnet-5"}}}' >"$work/providers.json"
provider_output="$(MISA_CONFIG="$work/providers.json" "$MISA_BIN" </dev/null)"
[ -z "$provider_output" ]

# Terminal/process ownership stays native while ordinary Lua composition and
# source loading remain available; decoded null retains its sentinel.
cat >"$work/sandbox.lua" <<'LUA'
assert(os == nil and io == nil and print == nil and debug == nil)
assert(type(package) == "table" and package.loadlib == nil and type(require) == "function")
assert(type(load) == "function" and type(loadstring) == "function" and type(loadfile) == "function" and type(dofile) == "function")
assert(ffi == nil and jit == nil)
local ok = pcall(require, "ffi"); assert(not ok)
package.preload["misa.test.module"] = function() return {answer=42} end
assert(require("misa.test.module").answer == 42)
return {setup=function(context)
  assert(context.config.missing == misa.json_null)
  misa.reg_event("app/start", function() return {fx={{type="app/quit"}}} end)
end}
LUA
printf '{"extensions":["%s"],"config":{"missing":null}}' "$work/sandbox.lua" >"$work/sandbox.json"
MISA_CONFIG="$work/sandbox.json" "$MISA_BIN"

# Event threading, interceptor direction, derived cofx, fx translation, one view,
# and registration sealing are exercised without extension-owned output.
cat >"$work/contracts.lua" <<'LUA'
return { setup = function()
  misa.reg_cofx("policy", function() return "derived" end)
  misa.reg_cofx("ordered", function(cofx) return cofx.policy .. ":ordered" end)
  misa.reg_interceptor({ id="trace", before=function(tx) tx.db.order=tx.db.order or {}; tx.db.order[#tx.db.order+1]="before"; return tx end,
    after=function(tx) tx.db.order[#tx.db.order+1]="after"; return tx end })
  misa.reg_event("app/start", function(db,event,cofx)
    assert(cofx.config.nested.value == 7)
    db.order[#db.order+1]="first:"..cofx.ordered
    return {db=db,fx={{type="test/next"}}}
  end)
  misa.reg_event("app/start", function(db) db.order[#db.order+1]="second"; return {db=db} end)
  misa.reg_fx("test/next", function() return {type="dispatch",event={type="test/done"}} end)
  misa.reg_event("test/done", function(db, event, cofx)
    assert(cofx.config.nested.value == 7 and cofx.argv[1] == "original")
    local ok,err=pcall(function() misa.reg_event("late",function() end) end)
    assert(not ok and err:find("sealed"))
    return {fx={{type="view/commit",lines={{spans={{text=table.concat(db.order,","),style="plain"}}}}},{type="app/quit"}}}
  end)
  misa.reg_view(function() return {lines={},cursor=nil} end)
end }
LUA
printf '{"extensions":["%s"],"config":{"nested":{"value":7}}}' "$work/contracts.lua" >"$work/contracts.json"
[ "$(MISA_CONFIG="$work/contracts.json" "$MISA_BIN" original)" = 'before,first:derived:ordered,second,after,before' ]

stage=semantic-components
cat >"$work/semantic-components.lua" <<'LUA'
return {setup=function()
  misa.reg_theme("test",{plain="plain"})
  misa.reg_animation("pulse",{frames={"one","two"}})
  misa.reg_component("test.first",{render=function() return {lines={{spans={{text="first",style="plain"}}}}} end})
  misa.reg_component("test.second",{render=function() return {lines={{spans={{text="swapped",style="plain"}}}}} end})
  misa.reg_event("app/start",function(db)
    assert(misa.render_component(db,"test.role",{}).lines[1].spans[1].text=="first")
    assert(misa.animation_frame(db,nil,1)=="two")
    return {fx={{type="dispatch",event={type="components/swap",role="test.role",implementation="test.second"}},{type="dispatch",event={type="test/render"}}}}
  end)
  misa.reg_event("test/render",function(db)
    return {fx={{type="view/commit",lines=misa.render_component(db,"test.role",{}).lines},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","animations","components","%s"],"config":{"themes":{"default":"test"},"animations":{"default":"pulse"},"components":{"roles":{"test.role":"test.first"}}}}' "$work/semantic-components.lua" >"$work/semantic-components.json"
[ "$(MISA_CONFIG="$work/semantic-components.json" "$MISA_BIN")" = swapped ]

stage=multi-timer
cat >"$work/multi-timer.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function(db)
    db.ticks={a=0,b=0}; return {db=db,fx={
      {type="timer/start",id="a",interval_ms=10,completion="tick/a"},
      {type="timer/start",id="b",interval_ms=10,completion="tick/b"},
    }}
  end)
  local function tick(db,name)
    db.ticks[name]=db.ticks[name]+1
    if db.ticks.a>0 and db.ticks.b>0 then return {db=db,fx={
      {type="timer/stop",id="a"},{type="timer/stop",id="b"},
      {type="view/commit",lines={{spans={{text="timers",style="plain"}}}}},{type="app/quit"},
    }} end
    return {db=db}
  end
  misa.reg_event("tick/a",function(db) return tick(db,"a") end)
  misa.reg_event("tick/b",function(db) return tick(db,"b") end)
end}
LUA
printf '{"extensions":["%s"],"config":{}}' "$work/multi-timer.lua" >"$work/multi-timer.json"
[ "$(MISA_CONFIG="$work/multi-timer.json" "$MISA_BIN")" = timers ]

stage=clear-state
cat >"$work/clear-state.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function(db)
    return {db=db,fx={
      {type="dispatch",event={type="agent/usage",usage={input_tokens=9,output_tokens=3},last_usage={input_tokens=7,output_tokens=2}}},
      {type="dispatch",event={type="agent/reset"}},
    }}
  end)
  misa.reg_event("agent/status",function(db,event)
    if event.last_usage==nil then return end
    assert(next(db.agent.last_usage)==nil,"agent last usage survived clear")
    assert(next(db.status.last_usage)==nil,"status context survived clear")
    return {db=db,fx={{type="view/commit",lines={{spans={{text="cleared",style="plain"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["status","messages","agent","%s"],"config":{}}' "$work/clear-state.lua" >"$work/clear-state.json"
[ "$(MISA_CONFIG="$work/clear-state.json" "$MISA_BIN")" = cleared ]

stage=unicode-layout
cat >"$work/unicode-layout.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function(db)
    assert(misa.layout.width("é界😀")==5,"cell width disagrees with terminal semantics")
    local wrapped=misa.layout.wrap_spans({{spans={{text="é界x",style="plain"}}}},3)
    assert(#wrapped==2 and wrapped[1].spans[1].text=="é界" and wrapped[2].spans[1].text=="x","Unicode span wrapping split or mismeasured text")
    assert(misa.layout.width(misa.layout.fit("界",4))==4,"cell fitting did not pad by cells")
    local picker=misa.render_component(db,"picker",{state={title="Pick",query="",active=1,selected=nil,panels={{title="Choices",filtered={{value="long",label="界界界界 wrapped tail",description="option"}},highlight=1}}},visible_panels=1,choice_room=1,first_index=function() return 1 end,panel_hint="tab",favorite_hint="",option_hints={{"alt+1"}}},{columns=28,available_lines=8})
    local text=""; for _,line in ipairs(picker.lines) do for _,part in ipairs(line.spans) do text=text..part.text end end
    assert(text:find("tail",1,true),"picker option was truncated instead of wrapped")
    return {fx={{type="view/commit",lines={{spans={{text="unicode layout",style="plain"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["components","layout","component.picker","%s"]}' "$work/unicode-layout.lua" >"$work/unicode-layout.json"
[ "$(MISA_CONFIG="$work/unicode-layout.json" "$MISA_BIN")" = 'unicode layout' ]

stage=independent-visual-swap
cat >"$work/visual-swap.lua" <<'LUA'
return {setup=function()
  misa.reg_component("test.status",{render=function() return {lines={{spans={{text="independent status",style="plain"}}}}} end})
  misa.reg_component("test.chrome",{render=function() return {lines={{spans={{text="independent chrome",style="plain"}}}}} end})
  misa.reg_event("app/start",function() return {fx={{type="dispatch",event={type="components/swap",role="status.metrics",implementation="test.status"}},{type="dispatch",event={type="test/status-swapped"}}}} end)
  misa.reg_event("test/status-swapped",function(db)
    assert(misa.render_component(db,"status.metrics",{metrics={}}).lines[1].spans[1].text=="independent status")
    assert(misa.render_component(db,"root.header",{}).lines[1].spans[1].text=="misa","status swap changed chrome")
    return {fx={{type="dispatch",event={type="components/swap",role="root.header",implementation="test.chrome"}},{type="dispatch",event={type="test/chrome-swapped"}}}}
  end)
  misa.reg_event("test/chrome-swapped",function(db)
    assert(misa.render_component(db,"status.metrics",{metrics={}}).lines[1].spans[1].text=="independent status","chrome swap changed status")
    return {fx={{type="view/commit",lines=misa.render_component(db,"root.header",{}).lines},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["components","component.status","component.chrome","%s"],"config":{"components":{"persist":false}}}' "$work/visual-swap.lua" >"$work/visual-swap.json"
[ "$(MISA_CONFIG="$work/visual-swap.json" "$MISA_BIN")" = 'independent chrome' ]

stage=message-redaction
cat >"$work/message-redaction.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function()
    return {fx={{type="dispatch",event={type="transcript/tool-call",id="call",name="demo",arguments={token="secret",value="abcdef"}}}}}
  end)
  misa.reg_event("transcript/tool-call",function(db)
    local args=db.messages.transcript[#db.messages.transcript].arguments
    assert(args.token=="[redacted]" and args.value=="abc… [truncated 3 bytes]")
    return {fx={{type="view/commit",lines={{spans={{text="redacted",style="plain"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","%s"],"config":{"messages":{"verbose":true,"max_string":3}}}' "$work/message-redaction.lua" >"$work/message-redaction.json"
[ "$(MISA_CONFIG="$work/message-redaction.json" "$MISA_BIN")" = redacted ]

stage=semantic-transcript
cat >"$work/semantic-transcript.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={
    {type="dispatch",event={type="transcript/user",text="# Heading with **emphasis** and `code`"}},
    {type="dispatch",event={type="transcript/assistant",content={{type="thinking",text="private"}},request_id="thought"}},
    {type="dispatch",event={type="transcript/tool-call",id="call",name="demo",arguments={value="detail"}}},
    {type="dispatch",event={type="transcript/tool-result",id="call",text="result"}},
  }} end)
  misa.reg_event("transcript/tool-result",function(db)
    local lines=misa.transcript_projection(db,{interactive=true,columns=32})
    assert(lines[1].spans[1].text=="▏ ","message box margin is missing")
    local bold=false; for _,line in ipairs(lines) do for _,span in ipairs(line.spans) do if span.style=="bold" then bold=true end end end
    assert(bold,"markdown did not produce semantic emphasis")
    assert(lines[#lines].spans[1].text:find("▏ Tool result",1,true),"tool result is not visibly collapsed")
    return {fx={{type="view/commit",lines={{spans={{text="semantic"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","%s"]}' "$work/semantic-transcript.lua" >"$work/semantic-transcript.json"
[ "$(MISA_CONFIG="$work/semantic-transcript.json" "$MISA_BIN")" = semantic ]

stage=lua-review-regressions
cat >"$work/review-regressions.lua" <<'LUA'
return {setup=function()
  misa.reg_component("test.mutating",{render=function(model) model.text="changed"; return {lines={{spans={{text="plain then **bold** and é界",style="plain"}}}}} end})
  misa.reg_event("app/start",function(db)
    local failures={
      pcall(function() misa.reg_component("late.component",{render=function() return {lines={}} end}) end),
      pcall(function() misa.reg_theme("late",{}) end),
      pcall(function() misa.reg_animation("late",{frames={"x"}}) end),
    }
    assert(not failures[1] and not failures[2] and not failures[3],"semantic registries did not seal at app/start")
    db.marker="original"
    local model={text="original"}; misa.render_component(db,"test.role",model,{})
    assert(db.marker=="original" and model.text=="original","component projection mutated canonical input")
    return {db=db,fx={{type="dispatch",event={type="agent/status",status="working"}},{type="dispatch",event={type="terminal/input",kind="text",text="ignored"}},{type="dispatch",event={type="terminal/input",kind="enter"}}}}
  end)
  misa.reg_event("terminal/input",function(db,event)
    assert(db.editor.text=="","busy input changed or submitted the editor")
    if event.kind=="enter" then return {fx={{type="view/commit",lines={{spans={{text="review regressions",style="plain"}}}}},{type="app/quit"}}} end
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","animations","animation.default","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui","%s"],"config":{"components":{"roles":{"test.role":"test.mutating"}}}}' "$work/review-regressions.lua" >"$work/review-regressions.json"
[ "$(MISA_CONFIG="$work/review-regressions.json" "$MISA_BIN")" = 'review regressions' ]

cat >"$work/transcript-order.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={{type="dispatch",event={type="transcript/assistant",request_id="ordered",content={
    {type="text",text="ordinary **bold**"},{type="thinking",text="thought"},{type="tool_call",id="call",name="demo",arguments={x=1}},{type="text",text="tail"},
  }}}}} end)
  misa.reg_event("transcript/assistant",function(db)
    local transcript=db.messages.transcript
    assert(#transcript==4 and transcript[1].kind=="assistant" and transcript[2].kind=="thinking" and transcript[3].kind=="tool_call" and transcript[4].kind=="assistant","normalized block order changed")
    local before=transcript[3].detail
    local lines=misa.transcript_projection(db,{interactive=true,columns=80})
    assert(transcript[3].detail==before,"transcript projection mutated its model")
    local bold=false; for _,line in ipairs(lines) do for _,item in ipairs(line.spans) do if item.text=="bold" and item.style=="bold" then bold=true end end end
    assert(bold,"inline Markdown after ordinary text was not parsed")
    return {fx={{type="view/commit",lines={{spans={{text="ordered",style="plain"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","%s"],"config":{"messages":{"verbose":true}}}' "$work/transcript-order.lua" >"$work/transcript-order.json"
[ "$(MISA_CONFIG="$work/transcript-order.json" "$MISA_BIN")" = "$(printf 'ordinary **bold**tail\nordered')" ]

cat >"$work/unserializable.lua" <<'LUA'
return {setup=function()
  misa.reg_request_options_serializer("test.transport",{accepts=function(name) return name=="known" end,serialize=function(target,name,value) target[name]=value; return true end})
  misa.reg_model({id="test/model",provider="test",model="model",api={request_options_serializer="test.transport",request_options={unknown={default="selected"}}}})
  misa.reg_fx("provider.test",function() error("blocked request reached provider") end)
end}
LUA
printf '{"extensions":["components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","agent","editor","ui","%s"],"config":{"models":{"default":"test/model"}}}' "$work/unserializable.lua" >"$work/unserializable.json"
[ "$(MISA_CONFIG="$work/unserializable.json" "$MISA_BIN" hello)" = 'request options cannot be serialized: unknown' ]

cat >"$work/picker-hints.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={{type="dispatch",event={type="picker/open",id="hints",token="hints:1",title="Hints",completion="hints/done",items={{value="one",label="One"}}}}}} end)
  misa.reg_event("picker/open",function(db)
    local layers=misa.view_layers(db,{terminal={columns=80,lines=20},available_lines=18})
    local found=false; for _,line in ipairs(layers[1].lines) do for _,item in ipairs(line.spans) do if item.text:find("alt+z",1,true) then found=true end end end
    assert(found,"configured picker hint disappeared")
    return {fx={{type="view/commit",lines={{spans={{text="hints",style="plain"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["keybindings","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","picker","picker_view","%s"],"config":{"keybindings":{"picker":{"option_1_1":["alt+z"]}}}}' "$work/picker-hints.lua" >"$work/picker-hints.json"
[ "$(MISA_CONFIG="$work/picker-hints.json" "$MISA_BIN")" = hints ]

stage=persistent-state
cat >"$work/state.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function()
    return {fx={{type="state/load",namespace="integration",completion="state/loaded"}}}
  end)
  misa.reg_event("state/loaded",function(_,event)
    assert(event.namespace=="integration")
    if event.data==misa.json_null then
      return {fx={{type="state/save",namespace="integration",data={count=7,nested={ok=true}}},{type="app/quit"}}}
    end
    assert(event.data.count==7 and event.data.nested.ok==true)
    return {fx={{type="view/commit",lines={{spans={{text="state loaded"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["%s"]}' "$work/state.lua" >"$work/state.json"
MISA_CONFIG="$work/state.json" "$MISA_BIN"
[ "$(stat -c %a "$MISA_STATE_FILE")" = 600 ]
[ "$(MISA_CONFIG="$work/state.json" "$MISA_BIN")" = 'state loaded' ]
grep -F '"integration"' "$MISA_STATE_FILE" >/dev/null
printf '{"version":2,"namespaces":{}}' >"$work/invalid-state.json"
if MISA_STATE_FILE="$work/invalid-state.json" MISA_CONFIG="$work/empty.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F InvalidState "$work/error" >/dev/null

cat >"$work/fake.json" <<'EOF'
{"extensions":["provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["fake response\n"]}}}}
EOF
[ "$(MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello world)" = 'fake response' ]
MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello >"$work/exact-output"
printf 'fake response\n' >"$work/expected-output"
cmp "$work/expected-output" "$work/exact-output"

stage=streaming-lifecycle
cat >"$work/stream-check.lua" <<'LUA'
return {setup=function()
  misa.reg_event("agent/completed",function(db)
    assert(db.agent.usage.input_tokens==2 and db.agent.usage.output_tokens==3,"stream usage was not finalized")
    local content=db.agent.messages[2].content
    assert(content[1].type=="text" and content[1].text=="stream ")
    assert(content[2].type=="thinking" and content[2].text=="private")
    assert(content[3].type=="text" and content[3].text=="works")
  end)
end}
LUA
printf '{"extensions":["provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[{"stream":[{"type":"text","text":"stream "},{"type":"thinking","text":"private"},{"type":"text","text":"works"}],"usage":{"input_tokens":2,"output_tokens":3}}]}}}}' "$work/stream-check.lua" >"$work/stream.json"
[ "$(MISA_CONFIG="$work/stream.json" "$MISA_BIN" hello)" = 'stream works' ]

cat >"$work/interrupted-check.lua" <<'LUA'
return {setup=function()
  misa.reg_event("agent/completed",function(db)
    assert(#db.agent.messages==1,"interrupted assistant response entered provider history")
    local transcript=db.messages.transcript
    assert(transcript[#transcript-1].interrupted==true and transcript[#transcript-1].text=="partial","partial transcript was not retained")
  end)
end}
LUA
printf '{"extensions":["provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[{"stream":[{"type":"text","text":"partial"}],"error":"stream interrupted"}]}}}}' "$work/interrupted-check.lua" >"$work/interrupted.json"
[ "$(MISA_CONFIG="$work/interrupted.json" "$MISA_BIN" hello)" = 'stream interrupted' ]

# util-linux script(1) gives the real binary a controlling pseudo-terminal.
# Keep this conditional because script flags and signal/pty behavior differ on
# non-Linux hosts; unit coverage still exercises the portable mechanisms.
if [ "$(uname -s)" = Linux ] && command -v script >/dev/null 2>&1 && command -v stty >/dev/null 2>&1 && script --version 2>/dev/null | grep -q util-linux; then
  stage=interactive-pty
  cat >"$work/pty.lua" <<'LUA'
return {setup=function()
  local request_id="pty-operation"
  misa.reg_event("app/start",function(_,_,cofx)
    local producer="printf '{\"part\":1}\\n'; sleep 0.70; printf '{\"part\":2}\\n'"
    if cofx.config.cancel then producer="echo $$ >'"..cofx.config.pid.."'; printf '{\"part\":1}\\n'; sleep 10" end
    return {fx={
      {type="process/run",argv={"/bin/sh","-c",producer},stdout_format="json_lines_stream",completion="pty/stream",id=request_id},
      {type="terminal/read"},
    }}
  end)
  misa.reg_event("pty/stream",function(_,event)
    if event.phase=="data" and #(event.records or {})>0 then
      return {fx={{type="view/commit",lines={{spans={{text="partial transcript"}}}}}}}
    end
    if event.phase=="end" then
      local text=event.message=="Canceled" and "cancelled child" or "producer completed"
      return {fx={{type="view/commit",lines={{spans={{text=text}}}}},{type="app/quit"}}}
    end
  end)
  misa.reg_event("terminal/resize",function(_,event)
    return {fx={{type="view/commit",lines={{spans={{text="resized "..event.columns.."x"..event.lines}}}}}}}
  end)
  misa.reg_event("terminal/input",function(_,event)
    if event.kind=="ctrl_c" then return {fx={{type="operation/cancel",id=request_id}}} end
    return {fx={{type="terminal/read"}}}
  end)
end}
LUA
  cat >"$work/pty-runner" <<SH
#!/bin/sh
stty rows 12 cols 40 </dev/tty
(sleep 0.20; stty rows 20 cols 60 </dev/tty; kill -WINCH "\$\$") &
exec "$MISA_BIN"
SH
  chmod +x "$work/pty-runner"

  printf '{"extensions":["%s"],"config":{}}' "$work/pty.lua" >"$work/pty-redraw.json"
  MISA_CONFIG="$work/pty-redraw.json" TERM=xterm script -qefc "$work/pty-runner" /dev/null </dev/null >"$work/pty-redraw.out"
  pty_redraw=$(cat "$work/pty-redraw.out")
  esc=$(printf '\033')
  case "$pty_redraw" in
    *"${esc}[?1049h"*"partial transcript"*"resized 60x20"*"producer completed"*"${esc}[?1049l"*) ;;
    *) echo "PTY redraw/resize/screen lifecycle was not observed" >&2; exit 1 ;;
  esac

  printf '{"extensions":["%s"],"config":{"cancel":true,"pid":"%s"}}' "$work/pty.lua" "$work/child.pid" >"$work/pty-cancel.json"
  { sleep 0.30; printf '\003'; sleep 0.20; } | MISA_CONFIG="$work/pty-cancel.json" TERM=xterm script -qefc "$work/pty-runner" /dev/null >"$work/pty-cancel.out"
  pty_cancel=$(cat "$work/pty-cancel.out")
  case "$pty_cancel" in
    *"${esc}[?1049h"*"partial transcript"*"cancelled child"*"${esc}[?1049l"*) ;;
    *) echo "PTY Ctrl-C cancellation/screen cleanup was not observed" >&2; exit 1 ;;
  esac
  child_pid=$(cat "$work/child.pid")
  i=0
  while kill -0 "$child_pid" 2>/dev/null && [ "$i" -lt 20 ]; do sleep 0.05; i=$((i + 1)); done
  if kill -0 "$child_pid" 2>/dev/null; then echo "stream child survived cancellation" >&2; exit 1; fi
fi

stage=request-options
# Reasoning controls are derived from fake model metadata rather than a fixed
# application-wide level list.
cat >"$work/effort-cycle.json" <<'EOF'
{"extensions":["keybindings","provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","effort","agent","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"models":[{"id":"fake/default","model":"default","api":{"request_options":{"reasoning_effort":{"choices":["low","medium","high"],"default":"low"}}}}],"expect_request_options":{"reasoning_effort":"medium"},"expect_request_options_exact":true,"responses":["cycled"]}}}}
EOF
[ "$(printf '\033ehello\n' | MISA_CONFIG="$work/effort-cycle.json" "$MISA_BIN")" = cycled ]

# A model switch retains equivalent values, but falls back to the new model's
# default when the old value is unavailable.
cat >"$work/effort-fallback.json" <<'EOF'
{"extensions":["provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","agent","editor","ui"],"config":{"models":{"default":"fake/wide"},"providers":{"fake":{"models":[{"id":"fake/wide","model":"wide","api":{"request_options":{"reasoning_effort":{"choices":["low","high"],"default":"high"}}}},{"id":"fake/narrow","model":"narrow","api":{"request_options":{"reasoning_effort":{"choices":["low","medium"],"default":"medium"}}}}],"expect_request_options":{"reasoning_effort":"medium"},"expect_request_options_exact":true,"responses":["fallback"]}}}}
EOF
[ "$(printf '/model fake/narrow\nhello\n' | MISA_CONFIG="$work/effort-fallback.json" "$MISA_BIN")" = fallback ]

# Cycling is a no-op for a model without reasoning support.
cat >"$work/effort-unsupported.json" <<'EOF'
{"extensions":["keybindings","provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","effort","agent","editor","ui"],"config":{"models":{"default":"fake/plain"},"providers":{"fake":{"models":[{"id":"fake/plain","model":"plain"}],"expect_request_options":{},"expect_request_options_exact":true,"responses":["plain"]}}}}
EOF
[ "$(printf '\033ehello\n' | MISA_CONFIG="$work/effort-unsupported.json" "$MISA_BIN")" = plain ]

# Missing generic required metadata blocks provider submission and reports a
# structured request-readiness problem through the harness transcript.
cat >"$work/required-option.json" <<'EOF'
{"extensions":["provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","agent","editor","ui"],"config":{"models":{"default":"fake/required"},"providers":{"fake":{"models":[{"id":"fake/required","model":"required","api":{"request_options":{"region":{"required":true,"choices":["east","west"]}}}}],"responses":["must not run"]}}}}
EOF
[ "$(MISA_CONFIG="$work/required-option.json" "$MISA_BIN" hello)" = 'required request options are missing: region' ]

stage=command-completion
cat >"$work/command-completion.lua" <<'LUA'
return {setup=function()
  misa.reg_command({name="/ping",description="test generic completion",event="test/ping"})
  misa.reg_event("test/ping",function()
    return {fx={{type="view/commit",lines={{spans={{text="pong"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["components","layout","component.message","component.editor","component.picker","component.status","component.chrome","editor","ui","%s"]}' "$work/command-completion.lua" >"$work/command-completion.json"
[ "$(printf 'discard me\003/p\t\n' | MISA_CONFIG="$work/command-completion.json" "$MISA_BIN")" = pong ]
[ -z "$(printf '\004' | MISA_CONFIG="$work/command-completion.json" "$MISA_BIN")" ]

stage=generic-picker
cat >"$work/generic-picker.lua" <<'LUA'
return {setup=function()
  misa.reg_command({name="/choose",description="test generic picker",event="test/choose"})
  misa.reg_command({name="/panels",description="test picker panels",event="test/panels"})
  misa.reg_event("test/choose",function(db)
    return {db=db,fx={{type="dispatch",event={type="picker/open",id="test",token="test:1",title="choice",selected="alpha",completion="test/chosen",items={
      {value="alpha",label="Alpha"},{value="beta/path",label="Beta"},
    }}}}}
  end)
  misa.reg_event("test/panels",function(db)
    return {db=db,fx={{type="dispatch",event={type="picker/open",id="panels",token="panels:1",title="panels",completion="test/chosen",panels={
      {id="one",title="One",items={{value="alpha"}}},
      {id="two",title="Two",items={{value="beta/path"}}},
      {id="three",title="Three",items={{value="gamma"}}},
    }}}}}
  end)
  misa.reg_event("test/chosen",function(_,event)
    local text=event.cancelled and ("cancelled "..event.picker_token) or event.value
    return {fx={{type="view/commit",lines={{spans={{text=text}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["keybindings","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","picker","picker_view","editor","ui","%s"]}' "$work/generic-picker.lua" >"$work/generic-picker.json"
[ "$(printf '/choose\nbeta\n' | MISA_CONFIG="$work/generic-picker.json" "$MISA_BIN")" = 'beta/path' ]
# The second visible panel uses the second positional Alt-key bank.
[ "$(printf '/panels\n\033q' | COLUMNS=120 MISA_CONFIG="$work/generic-picker.json" "$MISA_BIN")" = 'beta/path' ]
printf '/choose\n\033' | MISA_CONFIG="$work/generic-picker.json" "$MISA_BIN" >"$work/picker-cancelled"
grep -E '^cancelled test:[0-9]+$' "$work/picker-cancelled" >/dev/null

stage=generic-command-choice
cat >"$work/generic-command-choice.lua" <<'LUA'
return {setup=function()
  misa.reg_command({name="/choose",description="generic command choice",event="test/choose",completion="things"})
  misa.reg_completion("things",{value="alpha-one",label="Alpha"})
  misa.reg_completion("things",{value="beta-two",label="Beta"})
  misa.reg_event("test/choose",function(_,event)
    assert(event.arguments=="beta-two" and event.resumed_choice==true)
    return {fx={{type="view/commit",lines={{spans={{text=event.arguments}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["fuzzy","keybindings","command_choice","preferences","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","picker","picker_view","editor","ui","%s"]}' "$work/generic-command-choice.lua" >"$work/generic-command-choice.json"
[ "$(printf '/choose\ntwo beta\n' | MISA_CONFIG="$work/generic-command-choice.json" "$MISA_BIN")" = beta-two ]
# Favoriting keeps the terminal read live, updates the picker, and persists.
[ "$(printf '/choose\ntwo beta\033v\n' | MISA_CONFIG="$work/generic-command-choice.json" "$MISA_BIN")" = beta-two ]
grep -F '"favorite": true' "$MISA_STATE_FILE" >/dev/null

stage=dynamic-models
cat >"$work/dynamic-models.lua" <<'LUA'
return {setup=function()
  misa.reg_model({id="dynamic/old",provider="dynamic",model="old",label="Old",context_window=10})
  misa.reg_event("app/start",function()
    return {fx={{type="dispatch",event={type="models/replace-provider",provider="dynamic",authoritative=true,models={{id="dynamic/new",model="new",label="New",context_window=20}}}}}}
  end)
  misa.reg_fx("provider.dynamic",function(effect)
    assert(effect.model=="new")
    return {type="dispatch",event={type="agent/result",id=effect.id,content={{type="text",text="dynamic model"}}}}
  end)
end}
LUA
printf '{"extensions":["%s","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"]}' "$work/dynamic-models.lua" >"$work/dynamic-models.json"
[ "$(MISA_CONFIG="$work/dynamic-models.json" "$MISA_BIN" test)" = 'dynamic model' ]

stage=model-picker-filter
cat >"$work/model-picker-filter.lua" <<'LUA'
return {setup=function()
  misa.reg_model({id="picker/vendor/first",provider="picker",model="vendor/first",label="First"})
  misa.reg_model({id="picker/vendor/second",provider="picker",model="vendor/second",label="Second"})
  misa.reg_fx("provider.picker",function(effect)
    assert(effect.model=="vendor/second")
    return {type="dispatch",event={type="agent/result",id=effect.id,content={{type="text",text="picked "..effect.model}}}}
  end)
end}
LUA
printf '{"extensions":["%s","fuzzy","command_choice","preferences","picker","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","picker_view","messages","models","agent","editor","ui"],"config":{"models":{"default":"picker/vendor/first"}}}' "$work/model-picker-filter.lua" >"$work/model-picker-filter.json"
# Empty command invocation is intercepted generically, orderless fuzzy search
# highlights independently of the currently selected model, then resumes it.
[ "$(printf '/model\nsecond picker\nhello\n' | MISA_CONFIG="$work/model-picker-filter.json" "$MISA_BIN")" = 'picked vendor/second' ]

stage=unavailable-models
cat >"$work/unavailable-models.lua" <<'LUA'
return {setup=function()
  misa.reg_auth_provider({id="openai",model_provider="private",label="Private"})
  misa.reg_model({id="private/model",provider="private",model="model",label="Private model"})
end}
LUA
printf '{"extensions":["%s","auth","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"private/model"}}}' "$work/unavailable-models.lua" >"$work/unavailable-models.json"
[ "$(printf 'hello\n' | MISA_AUTH_FILE="$work/missing-auth.json" MISA_CONFIG="$work/unavailable-models.json" "$MISA_BIN")" = 'configured model is unavailable: private/model' ]

stage=agent-tool-loop
# The agent executes normalized tool calls, records results, and asks the
# provider to continue until it returns a final assistant message.
cat >"$work/tool.lua" <<'LUA'
return {setup=function()
  misa.reg_tool({name="echo",description="Echo text",input_schema={type="object"},effect="tool.echo"})
  misa.reg_fx("tool.echo",function(effect)
    assert(effect.arguments.value=="from tool")
    return {type="dispatch",event={type="tool/result",tool_call_id=effect.tool_call_id,text=effect.arguments.value}}
  end)
end}
LUA
printf '{"extensions":["json","provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[{"stream":[{"type":"tool_call","index":0,"id":"call-1","name":"echo","arguments_json_delta":"{\\\"value\\\":\\\"from "},{"type":"tool_call","index":0,"arguments_json_delta":"tool\\\"}"}]},"after tool"]}}}}' "$work/tool.lua" >"$work/tool-loop.json"
[ "$(MISA_CONFIG="$work/tool-loop.json" "$MISA_BIN" use tool)" = 'after tool' ]
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"value":"from tool"}}}' |
  MISA_CONFIG="$work/tool-loop.json" "$MISA_BIN" mcp >"$work/mcp-custom-output"
grep -F '"text":"from tool"' "$work/mcp-custom-output" >/dev/null

stage=cross-provider-tool-replay
cat >"$work/cross-provider.lua" <<'LUA'
return {setup=function()
  misa.reg_model({id="alpha/model",provider="alpha",model="model"})
  misa.reg_model({id="beta/model",provider="beta",model="model"})
  misa.reg_tool({name="switch",description="switch providers",input_schema={type="object"},effect="tool.switch"})
  misa.reg_fx("provider.alpha",function(effect)
    return {
      {type="dispatch",event={type="agent/stream-start",id=effect.id}},
      {type="dispatch",event={type="agent/stream-delta",id=effect.id,delta={type="tool_call",index=0,id="known",name="switch",arguments_json_delta="{\"value\":7,\"nested\":{\"ok\":true}}"}}},
      {type="dispatch",event={type="agent/stream-end",id=effect.id}},
    }
  end)
  misa.reg_fx("tool.switch",function(effect)
    assert(effect.arguments.value==7 and effect.arguments.nested.ok==true)
    return {type="dispatch",event={type="cross/switch"}}
  end)
  misa.reg_event("cross/switch",function()
    return {fx={{type="dispatch",event={type="model/select",id="beta/model"}},{type="dispatch",event={type="tool/result",tool_call_id="known",text="switched"}}}}
  end)
  local beta_calls=0
  misa.reg_fx("provider.beta",function(effect)
    beta_calls=beta_calls+1
    local known=effect.messages[2].content[1]
    assert(type(known.arguments)=="table" and known.arguments_json==nil)
    assert(misa.json.encode(known.arguments)=='{"nested":{"ok":true},"value":7}')
    local openai=misa.protocols.serialize_openai_messages(effect.messages)
    local anthropic=misa.protocols.serialize_anthropic_messages(effect.messages)
    assert(openai[2].tool_calls[1]["function"].arguments=='{"nested":{"ok":true},"value":7}')
    assert(anthropic[2].content[1].input.nested.ok==true)
    if beta_calls==1 then
      return {type="dispatch",event={type="agent/result",id=effect.id,content={{type="tool_call",id="unknown",name="missing",arguments_json="{\"portable\":true}"}}}}
    end
    local unknown=effect.messages[4].content[1]
    assert(type(unknown.arguments)=="table" and unknown.arguments.portable==true and unknown.arguments_json==nil)
    assert(openai[4].tool_calls[1]["function"].arguments=='{"portable":true}')
    assert(anthropic[4].content[1].input.portable==true)
    return {type="dispatch",event={type="agent/result",id=effect.id,content={{type="text",text="portable replay"}}}}
  end)
end}
LUA
printf '{"extensions":["json","protocol.openai","protocol.anthropic","%s","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"alpha/model"}}}' "$work/cross-provider.lua" >"$work/cross-provider.json"
[ "$(MISA_CONFIG="$work/cross-provider.json" "$MISA_BIN" replay)" = "portable replay" ]

stage=native-tools
cat >"$work/native-tools.json" <<JSON
{"extensions":["provider.fake","tool.files","tool.shell","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[[{"type":"tool_call","id":"write-1","name":"write_file","arguments":{"path":"$work/native-tool.txt","content":"alpha"}}],[{"type":"tool_call","id":"edit-1","name":"edit_file","arguments":{"path":"$work/native-tool.txt","old_text":"alpha","new_text":"beta"}}],[{"type":"tool_call","id":"read-1","name":"read_file","arguments":{"path":"$work/native-tool.txt"}}],[{"type":"tool_call","id":"list-1","name":"list_directory","arguments":{"path":"$work"}}],[{"type":"tool_call","id":"shell-1","name":"shell","arguments":{"command":"printf shell-ok"}}],"tools done"]}}}}
JSON
[ "$(MISA_CONFIG="$work/native-tools.json" "$MISA_BIN" use native tools)" = 'tools done' ]
[ "$(cat "$work/native-tool.txt")" = beta ]
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"write_file\",\"arguments\":{\"path\":\"$work/mcp-tool.txt\",\"content\":\"from mcp\"}}}" |
  MISA_CONFIG="$work/native-tools.json" "$MISA_BIN" mcp >"$work/mcp-output"
grep -F '"name":"read_file"' "$work/mcp-output" >/dev/null
grep -F '"name":"list_directory"' "$work/mcp-output" >/dev/null
grep -F '"isError":false' "$work/mcp-output" >/dev/null
[ "$(cat "$work/mcp-tool.txt")" = 'from mcp' ]

stage=ui-order
# Completion does not depend on ui being registered after agent/provider.
cat >"$work/ui-first.json" <<'EOF'
{"extensions":["ui","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","agent","models","editor","provider.fake"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["ui first"]}}}}
EOF
[ "$(MISA_CONFIG="$work/ui-first.json" "$MISA_BIN" hello)" = 'ui first' ]

# Provider-originated message strings are normalized at the transcript boundary,
# even when they did not pass through the process-output sanitizer.
cat >"$work/message-controls.json" <<'EOF'
{"extensions":["provider.fake","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["ok\tred\u001b[31m!\u001b[0m\u0001\u0085"]}}}}
EOF
[ "$(MISA_CONFIG="$work/message-controls.json" "$MISA_BIN" hello)" = 'ok red!' ]

# The standard editor inserts at its UTF-8 byte cursor, moves both directions,
# and backspaces one complete codepoint. The provider echoes the submitted text.
cat >"$work/echo-prompt" <<'SH'
#!/bin/sh
for prompt do :; done
printf '%s\n' "$prompt"
SH
chmod +x "$work/echo-prompt"
printf '{"extensions":["provider.command","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"command/default"},"providers":{"command":{"argv":["%s"]}}}}' "$work/echo-prompt" >"$work/editor.json"
printf 'ac\033[Db\033[D\033[Cd\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'abdc\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'aéx\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf '\033[200~ab\ncd\033[201~\033[D\177X\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ab\nXd\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"

stage=command-provider
# process/run receives direct argv. Shell metacharacters are one literal argument.
cat >"$work/provider" <<'SH'
#!/bin/sh
[ "$#" -eq 2 ] || exit 20
[ "$1" = "fixed;word" ] || exit 21
[ "$2" = '$(touch SHOULD_NOT_EXIST); it'"'"'s literal' ] || exit 22
# Exercise the full command-provider completion and view/commit path with
# output bytes that semantic terminal lines cannot accept unsanitized.
printf 'result\t%s:\377\n' "$2"
SH
chmod +x "$work/provider"
cat >"$work/command.json" <<EOF
{"extensions":["provider.command","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"command/default"},"providers":{"command":{"argv":["$work/provider","fixed;word"]}}}}
EOF
(cd "$work" && MISA_CONFIG="$work/command.json" "$MISA_BIN" '$(touch SHOULD_NOT_EXIST);' "it's" literal >"$work/command-output" && [ ! -e SHOULD_NOT_EXIST ])
printf 'result $(touch SHOULD_NOT_EXIST); it'"'"'s literal:�\n' >"$work/expected-command-output"
cmp "$work/expected-command-output" "$work/command-output"

stage=claude-provider
# Claude provider speaks the CLI's stream-json protocol, including the empty
# --tools argument, without requiring the Agent SDK package or copying auth.
cat >"$work/claude" <<'SH'
#!/bin/sh
saw_empty=false
saw_input=false
saw_mcp=false
saw_model=false
saw_partial=false
expect_mcp=false
expect_model=false
for arg do
  [ -z "$arg" ] && saw_empty=true
  [ "$arg" = "stream-json" ] && saw_input=true
  [ "$arg" = "--include-partial-messages" ] && saw_partial=true
  if [ "$expect_mcp" = true ]; then case "$arg" in *'"mcpServers"'*'"misa"'*) saw_mcp=true ;; esac; expect_mcp=false; fi
  if [ "$expect_model" = true ]; then [ "$arg" = "claude-sonnet-5" ] && saw_model=true; expect_model=false; fi
  [ "$arg" = "--mcp-config" ] && expect_mcp=true
  [ "$arg" = "--model" ] && expect_model=true
done
[ "$saw_empty" = true ] || exit 30
[ "$saw_input" = true ] || exit 31
[ "$saw_mcp" = true ] || exit 33
[ "$saw_model" = true ] || exit 34
[ "$saw_partial" = true ] || exit 35
input=$(cat)
case "$input" in *'"type":"user"'*) ;; *) exit 32 ;; esac
case "$input" in *'"content":"Continue this conversation.'*) ;; *) exit 32 ;; esac
printf '%s\n' '{"type":"system","subtype":"init","session_id":"test"}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"claude "}}}'
printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"result"}}}'
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"claude result"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"duplicate fallback","session_id":"test"}'
SH
chmod +x "$work/claude"
printf '{"extensions":["provider.claude","tool.files","components","layout","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","editor","ui"],"config":{"models":{"default":"claude/claude-sonnet-5"},"providers":{"claude":{"max_plan":true,"executable":"%s","mcp_command":"%s","mcp_arguments":["mcp","--config","%s"]}}}}' "$work/claude" "$MISA_BIN" "$work/claude.json" >"$work/claude.json"
claude_output="$(MISA_CONFIG="$work/claude.json" "$MISA_BIN" hello)"
if [ "$claude_output" != 'claude result' ]; then echo "claude output: $claude_output" >&2; exit 1; fi

stage=contract-errors
# Config and argv remain available in base cofx; plain output contains no ANSI.
cat >"$work/context.lua" <<'LUA'
return {setup=function(context)
  local ok = pcall(function() misa.reg_cofx("terminal", function() end) end)
  assert(not ok, "terminal cofx name must be reserved")
  misa.reg_event("app/start",function(db,event,cofx)
    assert(cofx.config.value==42 and cofx.argv[1]=='arg')
    assert(cofx.terminal.interactive == false and cofx.terminal.columns == 37 and cofx.terminal.lines == 11)
    return {fx={{type="view/commit",lines={{spans={{text="plain",style="accent"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["%s"],"config":{"value":42}}' "$work/context.lua" >"$work/context.json"
output="$(COLUMNS=37 LINES=11 MISA_CONFIG="$work/context.json" "$MISA_BIN" arg)"
[ "$output" = plain ]

# Unknown standard IDs, duplicate config, and traceback-bearing callback errors.
printf '{"extensions":["provider.unknown"]}' >"$work/bad.json"
if MISA_CONFIG="$work/bad.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "unknown standard extension ID 'provider.unknown'" "$work/error" >/dev/null
if "$MISA_BIN" --config "$work/empty.json" --config "$work/empty.json" 2>"$work/error"; then exit 1; fi
grep -F -- '--config may only be specified once' "$work/error" >/dev/null
cat >"$work/fail.lua" <<'LUA'
return {setup=function() misa.reg_event("app/start",function() error("exploded") end) end}
LUA
printf '{"extensions":["%s"]}' "$work/fail.lua" >"$work/fail.json"
if MISA_CONFIG="$work/fail.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F exploded "$work/error" >/dev/null
grep -F 'stack traceback:' "$work/error" >/dev/null
cat >"$work/legacy.lua" <<'LUA'
return {run=function() error("legacy run executed") end}
LUA
printf '{"extensions":["%s"]}' "$work/legacy.lua" >"$work/legacy.json"
if MISA_CONFIG="$work/legacy.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "field 'run' is obsolete" "$work/error" >/dev/null

cat >"$work/malformed-setup.lua" <<'LUA'
return {setup="not a function"}
LUA
printf '{"extensions":["%s"]}' "$work/malformed-setup.lua" >"$work/malformed-setup.json"
if MISA_CONFIG="$work/malformed-setup.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "field 'setup' must be a function" "$work/error" >/dev/null
cat >"$work/setup-trace.lua" <<'LUA'
return {setup=function() local function nested() error("setup exploded") end; nested() end}
LUA
printf '{"extensions":["%s"]}' "$work/setup-trace.lua" >"$work/setup-trace.json"
if MISA_CONFIG="$work/setup-trace.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "setup exploded" "$work/error" >/dev/null
grep -F "stack traceback:" "$work/error" >/dev/null

cat >"$work/late-effect.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start", function() return {fx={{type="app/quit"},{type="not/native"}}} end)
end}
LUA
printf '{"extensions":["%s"]}' "$work/late-effect.lua" >"$work/late-effect.json"
if MISA_CONFIG="$work/late-effect.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "UnknownNativeEffect" "$work/error" >/dev/null

# NUL is rejected at both extension and direct-command argument boundaries.
printf '{"extensions":["bad\\u0000.lua"]}' >"$work/nul-extension.json"
if MISA_CONFIG="$work/nul-extension.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "ExtensionContainsNul" "$work/error" >/dev/null
printf '{"extensions":["provider.command"],"config":{"providers":{"command":{"argv":["bad\\u0000arg"]}}}}' >"$work/nul-command.json"
if MISA_EXTENSION_DIR="${MISA_EXTENSION_DIR:-}" MISA_CONFIG="$work/nul-command.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
grep -F "command argv must not contain NUL" "$work/error" >/dev/null

# Lua conversion and framework recursion are bounded at 128 levels.
deep='{}'; i=0
while [ "$i" -lt 130 ]; do deep="{\"x\":$deep}"; i=$((i + 1)); done
printf '{"config":%s}' "$deep" >"$work/deep.json"
if MISA_CONFIG="$work/deep.json" "$MISA_BIN" 2>"$work/error"; then exit 1; fi
case "$(cat "$work/error")" in *nesting*|*depth*) ;; *) exit 1 ;; esac
:
