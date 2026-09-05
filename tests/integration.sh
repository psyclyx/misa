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
printf '%s' '{"extensions":["auth","protocol.openai","provider.openai","fuzzy","commands","choice_tree","choices","preferences","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","choice_layout","picker","picker_view","editor","ui"],"config":{"providers":{"openai":{"discover_models":false}}}}' >"$work/auth-ui.json"
[ "$(printf '/status op\t\n' | MISA_CONFIG="$work/auth-ui.json" "$MISA_BIN")" = 'logged out' ]
[ "$(printf '/status\nopenai\n' | MISA_CONFIG="$work/auth-ui.json" "$MISA_BIN")" = 'logged out' ]

stage=provider-composition
# Real provider declarations compose without credentials until they are used.
printf '%s' '{"extensions":["protocol.anthropic","provider.anthropic","provider.kimi","protocol.openai","provider.openai","provider.openrouter","provider.openai-codex","provider.claude","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"anthropic/claude-sonnet-5"}}}' >"$work/providers.json"
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
assert(type(misa.syntax) == "table" and type(misa.syntax.highlight) == "function")
assert(#misa.syntax.highlight("grammar_that_does_not_exist", "plain") == 0)
assert(not pcall(misa.syntax.highlight, {}, "plain"))
assert(not pcall(misa.syntax.highlight, "python", {}))
assert(not pcall(misa.syntax.highlight, "python", string.rep("x", 1024 * 1024 + 1)))
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
    return {fx={{type="view/commit",lines={{spans={{text=table.concat(db.order,","),style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
  misa.reg_view(function() return {lines={},cursor=nil} end)
end }
LUA
printf '{"extensions":["%s"],"config":{"nested":{"value":7}}}' "$work/contracts.lua" >"$work/contracts.json"
[ "$(MISA_CONFIG="$work/contracts.json" "$MISA_BIN" original)" = 'before,first:derived:ordered,second,after,before' ]

stage=native-clock
cat >"$work/clock.lua" <<'LUA'
return {setup=function()
  assert(not pcall(function() misa.reg_cofx("clock",function() end) end),"clock cofx name must be reserved")
  local first
  misa.reg_event("app/start",function(_,_,cofx)
    assert(cofx.clock.wall_ms>1000000000000 and cofx.clock.monotonic_ms>=0,"native clocks are not trustworthy milliseconds")
    first=cofx.clock.monotonic_ms; return {fx={{type="dispatch",event={type="clock/next"}}}}
  end)
  misa.reg_event("clock/next",function(_,_,cofx)
    assert(cofx.clock.monotonic_ms>=first,"monotonic coeffect went backwards")
    return {fx={{type="view/commit",lines={{spans={{text="clock",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["%s"]}' "$work/clock.lua" >"$work/clock.json"
[ "$(MISA_CONFIG="$work/clock.json" "$MISA_BIN")" = clock ]

stage=generic-indicators
cat >"$work/indicators.lua" <<'LUA'
return {setup=function()
  misa.reg_keybinding({context="test",action="cycle",default={"alt+x"}})
  misa.reg_indicator({id="important",label="Important",icon="!",value=function() return "yes" end})
  misa.reg_indicator({id="optional",label="Optional",icon="?",hotkey={context="test",action="cycle"},value=function() return "wide" end})
  misa.reg_event("app/start",function(db)
    local wide=misa.indicators_projection(db,{columns=80})[1].spans
    local labels,values,hotkey_text=0,0,""; for _,item in ipairs(wide) do
      if item.style.dim==true and item.text=="Important" then labels=labels+1 end
      if item.style.foreground=="default" and item.text=="yes" then values=values+1 end
      if item.style.dim==true and item.style.foreground=="cyan" then hotkey_text=hotkey_text..item.text end
    end
    assert(labels==1 and values==1 and hotkey_text=="⌥X","indicator semantic classes or structured hotkey are missing")
    local narrow=misa.indicators_projection(db,{columns=15})[1].spans; local text=""; for _,item in ipairs(narrow) do text=text..item.text end
    assert(text:find("Important",1,true) and not text:find("Optional",1,true),"indicator priority did not control narrow-width dropping")
    return {fx={{type="view/commit",lines={{spans={{text="indicators",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["keybindings","themes","theme.default","components","layout","indicators","component.status","%s"],"config":{"status":{"indicators":[{"id":"important","priority":100},{"id":"optional","hotkey":true,"priority":1}]}}}' "$work/indicators.lua" >"$work/indicators.json"
[ "$(MISA_CONFIG="$work/indicators.json" "$MISA_BIN")" = indicators ]

stage=semantic-components
cat >"$work/semantic-components.lua" <<'LUA'
return {setup=function()
  misa.reg_theme("test",{palette={text="default"},styles={plain={foreground="text"}}})
  assert(not pcall(function() misa.reg_theme("bad",{palette={oops="orange"},styles={}}) end),"invalid palette color was accepted")
  assert(not pcall(function() misa.reg_theme("foundationless",{palette={},styles={label={dim=true}}}) end),"foundationless theme was accepted")
  misa.reg_animation("pulse",{frames={"one","two"}})
  misa.reg_component("test.first",{render=function() return {lines={{spans={{text="first",style="plain"}}}}} end})
  misa.reg_component("test.second",{render=function() return {lines={{spans={{text="swapped",style="plain"}}}}} end})
  misa.reg_event("app/start",function(db)
    assert(misa.render_component(db,"test.role",{}).lines[1].spans[1].text=="first")
    assert(misa.theme_style(db,"syntax.escape").bold==true and misa.theme_style(db,"dialog.hint").dim==true,"complete standard theme fallbacks were not composed")
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
      {type="view/commit",lines={{spans={{text="timers",style={foreground="default"}}}}}},{type="app/quit"},
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
    return {db=db,fx={{type="view/commit",lines={{spans={{text="cleared",style={foreground="default"}}}}}},{type="app/quit"}}}
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
    assert(misa.layout.previous_boundary("éx",3)==0 and misa.layout.next_boundary("éx",0)==3,"combining grapheme boundaries diverged")
    assert(misa.layout.previous_boundary("👩‍💻x",11)==0 and misa.layout.next_boundary("👩‍💻x",0)==11,"ZWJ grapheme boundaries diverged")
    assert(misa.layout.previous_boundary("क्x",6)==0 and misa.layout.next_boundary("क्x",0)==6,"virama grapheme boundaries diverged")
    assert(misa.layout.previous_boundary("क्षx",9)==0 and misa.layout.next_boundary("क्षx",0)==9,"Devanagari conjunct boundaries diverged")
    local input=misa.render_component(db,"editor.input",{text="ab界é👩‍💻क्ष\nz",cursor=5},{columns=6})
    assert(#input.lines==3 and input.lines[1].spans[2].text=="ab界","input did not wrap on cell/grapheme boundaries")
    assert(input.cursor.row==2 and input.cursor.byte==2,"logical cursor did not map across a prompt-prefixed soft wrap")
    local narrow=misa.render_component(db,"editor.input",{text="界é",cursor=3},{columns=1})
    assert(#narrow.lines==2 and narrow.lines[1].spans[1].text=="" and narrow.cursor.row==2 and narrow.cursor.byte==0,"narrow input wrapping lost its cursor or prompt budget")
    local cjk=misa.render_component(db,"editor.input",{text="界",cursor=3},{columns=2})
    assert(cjk.lines[1].spans[2].text=="界" and cjk.cursor.byte==3,"narrow prompt hid a wide grapheme")
    local crlf=misa.layout.wrap_input("a\r\nb\rc",6,3,"> ","plain","accent")
    assert(#crlf.lines==3 and crlf.cursor.row==2 and crlf.cursor.byte==2,"CRLF cursor was not translated through newline normalization: "..#crlf.lines..","..crlf.cursor.row..","..crlf.cursor.byte)
    local inside_crlf=misa.layout.wrap_input("a\r\nb",6,2,"> ","plain","accent")
    assert(inside_crlf.cursor.row==2 and inside_crlf.cursor.byte==2,"cursor inside CRLF was not normalized coherently")
    local picker=misa.render_component(db,"picker",{x=0,width=28,height=8,input={title="Pick",text="",cursor=0},preview={lines={},height=0},columns={{id="all",title="Choices",width=28,active=true,rows={{value="long",label="界界界界 wrapped tail",description="option",marker=">",hotkey="alt+1",active=true}}}},panel_height=6,hints={}})
    local text=""; for _,line in ipairs(picker.lines) do for _,part in ipairs(line.spans) do text=text..part.text end end
    assert(text:find("tail",1,true),"picker option was truncated instead of wrapped")
    return {fx={{type="view/commit",lines={{spans={{text="unicode layout",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","component.editor","component.picker","%s"]}' "$work/unicode-layout.lua" >"$work/unicode-layout.json"
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
printf '{"extensions":["themes","theme.default","components","component.status","component.chrome","%s"],"config":{"components":{"persist":false}}}' "$work/visual-swap.lua" >"$work/visual-swap.json"
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
    return {fx={{type="view/commit",lines={{spans={{text="redacted",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","%s"],"config":{"messages":{"verbose":true,"max_string":3}}}' "$work/message-redaction.lua" >"$work/message-redaction.json"
[ "$(MISA_CONFIG="$work/message-redaction.json" "$MISA_BIN")" = redacted ]

stage=semantic-transcript
cat >"$work/semantic-transcript.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={
    {type="dispatch",event={type="transcript/user",text="# Heading with **emphasis**, *italics*, ~~gone~~, `code`, and [docs](https://example.test)"}},
    {type="dispatch",event={type="transcript/assistant",content={{type="thinking",text="private"}},request_id="thought"}},
    {type="dispatch",event={type="transcript/tool-call",id="call",name="demo",arguments={value="detail"}}},
    {type="dispatch",event={type="transcript/tool-result",id="call",text="result"}},
  }} end)
  misa.reg_event("transcript/tool-result",function(db)
    local lines=misa.transcript_projection(db,{interactive=true,columns=32})
    assert(lines[1].spans[1].text=="You" and lines[2].spans[1].text=="┃ ","message title/body separation is missing")
    local bold,italic,strike,linked,rails=false,false,false,false,{}
    for _,line in ipairs(lines) do for _,span in ipairs(line.spans) do
      if span.style.bold==true and span.style.foreground=="green" then bold=true end
      if span.style.italic==true then italic=true end
      if span.style.strikethrough==true then strike=true end
      if span.link=="https://example.test" then linked=true end
      if span.text=="┃ " then rails[span.style.foreground]=true end
    end end
    assert(bold and italic and strike and linked,"composed Markdown styles or links are missing")
    assert(rails.green and rails.magenta and rails.yellow,"semantic message rails did not retain distinct role colors")
    local sections=0; for _,block in ipairs(db.messages.blocks) do if block.kind=="tool_call" then sections=sections+1; assert(block.result=="result" and block.status=="success","tool result did not update its call section") end end
    assert(sections==1 and #db.messages.blocks==3,"matching tool result created an unrelated transcript block")
    return {fx={{type="view/commit",lines={{spans={{text="semantic"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","%s"]}' "$work/semantic-transcript.lua" >"$work/semantic-transcript.json"
[ "$(MISA_CONFIG="$work/semantic-transcript.json" "$MISA_BIN")" = semantic ]

stage=markdown-rendering
cat >"$work/markdown-rendering.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function(db)
    local highlighted=false; local native_highlight=misa.syntax.highlight
    misa.syntax.highlight=function(language,source) highlighted=true; return native_highlight(language,source) end
    local source="# live\n## second\n***both*** and ~~**gone**~~ and `x` [docs](https://example.test)\n\n- [ ] todo\n  continuation\n  2. nested\n\n> quoted\n\n---\n\n| left | centered | right |\n| :--- | :------: | ----: |\n| a | growing value | 7 |\n\n```bogus\nreturn 42\n"
    local document=misa.markdown.parse(source)
    assert(document.kind=="document" and document.blocks[1].kind=="heading" and document.blocks[1].level==1)
    local narrow=misa.markdown_view.render(document,{base="assistant",columns=30})
    local wide=misa.markdown_view.render(document,{base="assistant",columns=60})
    local text,narrow_top,wide_top="",nil,nil
    local saw={h1=false,h2=false,both=false,strike=false,box=false,continuation=false,quote=false,rule=false,table=false,code=false,link=false}
    for _,line in ipairs(narrow) do
      local line_text=""; for _,item in ipairs(line.spans) do
        line_text=line_text..item.text; text=text..item.text
        local styles={}; if type(item.style)=="table" then for _,token in ipairs(item.style) do styles[token]=true end end
        if item.text=="both" and styles.bold and styles.italic then saw.both=true end
        if item.text=="gone" and styles.bold and styles.strikethrough then saw.strike=true end
        if item.link=="https://example.test" and item.text=="docs" then saw.link=true end
      end
      if line_text:match("^█ ") then saw.h1=true end; if line_text:match("^▌ ") then saw.h2=true end
      if line_text:find("☐ ",1,true) then saw.box=true end; if line_text:match("^  continuation") then saw.continuation=true end
      if line_text:find("▏ ",1,true) then saw.quote=true end; if line_text==string.rep("─",30) then saw.rule=true end
      if line_text:find("┌",1,true) then saw.table=true; narrow_top=misa.layout.width(line_text) end
      if line_text:find("bogus",1,true) then saw.code=true end
    end
    for _,line in ipairs(wide) do local line_text=""; for _,item in ipairs(line.spans) do line_text=line_text..item.text end; if line_text:find("┌",1,true) then wide_top=misa.layout.width(line_text); break end end
    for feature,value in pairs(saw) do assert(value,"missing Markdown rendering: "..feature) end
    assert(highlighted,"fenced code did not invoke the syntax service")
    assert(not text:find("***",1,true) and not text:find("~~",1,true) and not text:find("[docs]",1,true),"Markdown delimiters leaked")
    assert(narrow_top and wide_top and narrow_top<=30 and wide_top>narrow_top,"table columns did not respond to streaming width")
    return {db=db,fx={{type="view/commit",lines={{spans={{text="markdown",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","%s"]}' "$work/markdown-rendering.lua" >"$work/markdown-rendering.json"
[ "$(MISA_CONFIG="$work/markdown-rendering.json" "$MISA_BIN")" = markdown ]

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
    local clipped=misa.ui_bound_frame({{spans={
      {text="a"},{text="👩‍",style={bold=true},link="https://one"},{text="💻z",style={underline=true},link="https://two"},
    }}},4,{row=1,byte=12})
    assert(#clipped.lines[1].spans==3 and clipped.lines[1].spans[2].text=="👩‍" and clipped.lines[1].spans[2].link=="https://one" and clipped.lines[1].spans[3].text=="💻" and clipped.lines[1].spans[3].link=="https://two","final clipping split a cross-span grapheme or erased styling/link boundaries")
    assert(clipped.cursor.byte==12,"final clipping produced an invalid grapheme cursor")
    local omitted=misa.ui_bound_frame({{spans={{text="a"},{text="👩‍"},{text="💻z"}}}},3,{row=1,byte=5})
    assert(#omitted.lines[1].spans==1 and omitted.lines[1].spans[1].text=="a" and omitted.cursor.byte==1,"clipped grapheme fragment or split cursor survived")
    return {db=db,fx={{type="dispatch",event={type="agent/status",status="working"}},{type="dispatch",event={type="terminal/input",kind="text",text="ignored"}},{type="dispatch",event={type="terminal/input",kind="enter"}}}}
  end)
  misa.reg_event("terminal/input",function(db,event)
    assert(db.editor.text=="","busy input changed or submitted the editor")
    if event.kind=="enter" then return {fx={{type="view/commit",lines={{spans={{text="review regressions",style={foreground="default"}}}}}},{type="app/quit"}}} end
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","animations","animation.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui","%s"],"config":{"components":{"roles":{"test.role":"test.mutating"}}}}' "$work/review-regressions.lua" >"$work/review-regressions.json"
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
    local bold=false; for _,line in ipairs(lines) do for _,item in ipairs(line.spans) do if item.text=="bold" and item.style.bold==true and item.style.foreground=="blue" then bold=true end end end
    assert(bold,"inline Markdown after ordinary text was not parsed")
    return {fx={{type="view/commit",lines={{spans={{text="ordered",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","%s"],"config":{"messages":{"verbose":true}}}' "$work/transcript-order.lua" >"$work/transcript-order.json"
[ "$(MISA_CONFIG="$work/transcript-order.json" "$MISA_BIN")" = "$(printf 'ordinary **bold**tail\nordered')" ]

cat >"$work/response-metadata.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={
    {type="dispatch",event={type="transcript/response-start",response_id="metadata",role="assistant"}},
    {type="dispatch",event={type="transcript/block-start",response_id="metadata",block_id="metadata/1",kind="assistant"}},
    {type="dispatch",event={type="transcript/block-delta",response_id="metadata",block_id="metadata/1",text="first"}},
    {type="dispatch",event={type="transcript/block-end",response_id="metadata",block_id="metadata/1"}},
    {type="dispatch",event={type="transcript/block-start",response_id="metadata",block_id="metadata/2",kind="tool_call",name="demo",call_id="call"}},
    {type="dispatch",event={type="transcript/block-delta",response_id="metadata",block_id="metadata/2",arguments_json_delta="{\"value\":"}},
    {type="dispatch",event={type="transcript/block-delta",response_id="metadata",block_id="metadata/2",arguments_json_delta="1}"}},
    {type="dispatch",event={type="transcript/block-end",response_id="metadata",block_id="metadata/2"}},
    {type="dispatch",event={type="transcript/block-start",response_id="metadata",block_id="metadata/3",kind="assistant"}},
    {type="dispatch",event={type="transcript/block-delta",response_id="metadata",block_id="metadata/3",text="second"}},
    {type="dispatch",event={type="transcript/block-end",response_id="metadata",block_id="metadata/3"}},
    {type="timer/start",id="metadata-delay",interval_ms=20,completion="test/metadata-finish"},
  }} end)
  misa.reg_event("test/metadata-finish",function() return {fx={{type="timer/stop",id="metadata-delay"},{type="dispatch",event={type="transcript/response-end",response_id="metadata",usage={output_tokens=12}}}}} end)
  misa.reg_event("transcript/response-end",function(db,event)
    if event.response_id~="metadata" then return end
    local tool=db.messages.blocks[2]; assert(tool.argument_chunks==nil and tool.argument_text=='{"value":1}',"final tool argument chunks were not compacted")
    local lines=misa.transcript_projection(db,{interactive=true,columns=80}); local rates=0
    for _,line in ipairs(lines) do for _,item in ipairs(line.spans) do if item.text:find("tok/s",1,true) then rates=rates+1 end end end
    assert(rates==1,"response tok/s metadata was missing or rendered more than once")
    return {fx={{type="view/commit",lines={{spans={{text="metadata"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","messages","%s"],"config":{"messages":{"verbose":true}}}' "$work/response-metadata.lua" >"$work/response-metadata.json"
[ "$(MISA_CONFIG="$work/response-metadata.json" "$MISA_BIN")" = "$(printf 'firstsecond\nmetadata')" ]

cat >"$work/unserializable.lua" <<'LUA'
return {setup=function()
  misa.reg_request_options_serializer("test.transport",{accepts=function(name) return name=="known" end,serialize=function(target,name,value) target[name]=value; return true end})
  misa.reg_model({id="test/model",provider="test",model="model",api={request_options_serializer="test.transport",request_options={unknown={default="selected"}}}})
  misa.reg_fx("provider.test",function() error("blocked request reached provider") end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","agent","commands","choice_tree","choices","editor","ui","%s"],"config":{"models":{"default":"test/model"}}}' "$work/unserializable.lua" >"$work/unserializable.json"
[ "$(MISA_CONFIG="$work/unserializable.json" "$MISA_BIN" hello)" = 'request options cannot be serialized: unknown' ]

cat >"$work/picker-hints.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={{type="dispatch",event={type="picker/open",id="hints",token="hints:1",title="Hints",completion="hints/done",items={{value="one",label="One"}}}}}} end)
  misa.reg_event("picker/open",function(db)
    local layers=misa.view_layers(db,{terminal={columns=80,lines=20},available_lines=18})
    local found=false; for _,line in ipairs(layers[1].lines) do local text=""; for _,item in ipairs(line.spans) do text=text..item.text end; if text:find("⌥Z",1,true) then found=true end end
    assert(found,"configured picker hint disappeared")
    return {fx={{type="view/commit",lines={{spans={{text="hints",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["keybindings","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","commands","choice_tree","choices","choice_layout","picker","picker_view","%s"],"config":{"keybindings":{"choices":{"option_1_1":["alt+z"]}}}}' "$work/picker-hints.lua" >"$work/picker-hints.json"
[ "$(MISA_CONFIG="$work/picker-hints.json" "$MISA_BIN")" = hints ]

stage=choice-contracts
cat >"$work/choice-contracts.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function(db)
    local ordered=misa.choice_session({title="Models",purpose="models",items={{value="vendor/one"}}},db)
    assert(ordered.view_ids[1]=="frecency" and ordered.view_ids[2]=="all","purpose view order was ignored")
    local registered=misa.choice_registered_views(ordered); local tree_available=false
    for _,item in ipairs(registered) do if item.value=="slash-prefix" then tree_available=true end end
    assert(tree_available,"model tree is not available as an opt-in replacement")

    local flat=misa.choice_session({title="Flat",purpose="generic",views={"all"},items={{value="vendor/model",label="vendor/model",description="vendor/model",search={"Hidden Label"}}},query="hidden"},db)
    local flat_rows=misa.choice_rows(flat,db)[1].rows
    assert(#flat_rows==1 and flat_rows[1].description==nil,"flat rows repeated redundant model text or lost hidden search")

    local tree=misa.choice_session({title="Tree",purpose="models",views={"slash-prefix"},tree_node="vendor/",items={{value="vendor/model",label="vendor/model",search="Hidden Label"}}},db)
    assert(misa.choice_rows(tree,db)[1].rows[1].label=="model","tree leaf repeated its full qualified name")
    local hidden=misa.choice_session({title="Tree",purpose="models",views={"slash-prefix"},items={{value="vendor/one",search="Secret Full Name"},{value="vendor/two"}},query="secret"},db)
    assert(#hidden.panels[1].items==1 and hidden.panels[1].items[1].tree_prefix=="vendor/","tree group search omitted hidden descendant fields")

    local command=misa.choice_session({title="Commands",purpose="command-completion",views={"slash-prefix"},tree_node="/",items={{value="/team/one"},{value="/team/two"}}},db)
    local entered=misa.choice_input(command,{kind="text",text="t"},db)
    assert(entered.consumed and command.tree.node=="/" and command.query=="t","tree typing expanded implicitly")
    local expanded=misa.choice_accept(command,command.panels[1].items[1],db)
    assert(expanded.tree_changed and command.tree.node=="/team/" and command.query=="","explicit tree expansion lost its breadcrumb")
    local backed=misa.choice_input(command,{kind="backspace"},db)
    assert(backed.tree_changed and command.tree.node=="/" and command.query=="","tree backspace was not an atomic parent transition")

    local panels=misa.choice_session({title="Panels",purpose="generic",view_definitions={{id="one",items={{value="first"}}},{id="two",items={{value="second"}}}}},db)
    assert(misa.choice_positional(panels,"option_2_1",1,5)==nil,"hidden positional bank activated")
    assert(misa.choice_positional(panels,"option_1_2",1,1)==nil,"hidden positional slot activated")
    assert(misa.choice_positional(panels,"option_2_1",2,5).value=="second","visible positional bank did not activate")
    local narrow=misa.choice_picker_layout(panels,db,{columns=57,lines=8})
    assert(narrow.panel_count==1 and narrow.targets.option_2_1==nil,"nonrendered picker panel retained an active hotkey")
    local wrapped=misa.choice_session({title="Wrapped",views={"all"},items={{value="one",label=string.rep("long ",20)},{value="two"}}},db)
    local visible=misa.choice_picker_layout(wrapped,db,{columns=30,lines=6})
    assert(#visible.columns[1].rows==0 and visible.targets.option_1_1==nil,"oversized wrapped picker row or target escaped the panel budget")
    local no_budget=misa.choice_picker_layout(flat,db,{columns=30,available_lines=2})
    assert(#no_budget.columns[1].rows==0 and next(no_budget.targets)==nil,"zero panel budget exposed a row or target")
    local prefixed=misa.choice_session({title="Arguments",views={"all"},input_prefix="/model ",query="vendor/",items={{value="vendor/model"}}},db)
    local prefixed_layout=misa.choice_picker_layout(prefixed,db,{columns=40,lines=8})
    assert(prefixed_layout.input.text=="/model vendor/" and prefixed_layout.input.cursor==#prefixed_layout.input.text,"narrowed picker omitted its canonical command prefix")

    local parent=misa.choice_session({title="Parent",purpose="generic",view_definitions={{id="parent",items={{value="parent"}}}}},db)
    assert(parent.selected==nil and parent.preference_scope==nil and parent.custom_views~=nil)
    misa.choice_accept(parent,{narrow={title="Child",purpose="generic",items={{value="child"}},selected="child",preference_scope="child-scope"}},db)
    assert(parent.selected=="child" and parent.preference_scope=="child-scope" and parent.custom_views==nil,"parent-only narrowing state leaked into child")
    misa.choice_input(parent,{action="cancel"},db)
    assert(parent.selected==nil and parent.preference_scope==nil and parent.custom_views~=nil and parent.view_ids[1]=="parent","narrow frame did not restore absent and custom state exactly")
    assert(misa.choice_action({kind="key",key="alt+x"})=="open_overlay","configured shared action was not resolved")
    assert(misa.choice_hint("option_1_1")=="alt+z","configured shared positional hint was not resolved")

    misa.choice_replace_view(ordered,"slash-prefix",db)
    local fresh=misa.choice_session({title="Models",purpose="models",items={{value="vendor/one"}}},db)
    assert(fresh.view_ids[1]=="frecency","view replacement leaked beyond its session")
    return {fx={{type="view/commit",lines={{spans={{text="choice contracts",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["fuzzy","keybindings","layout","commands","choice_tree","choices","choice_layout","%s"],"config":{"choices":{"purposes":{"models":["frecency","all"]}},"keybindings":{"choices":{"open_overlay":["alt+x"],"option_1_1":["alt+z"]}}}}' "$work/choice-contracts.lua" >"$work/choice-contracts.json"
[ "$(MISA_CONFIG="$work/choice-contracts.json" "$MISA_BIN")" = 'choice contracts' ]

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
# State I/O is lazy and worker-owned; malformed persistence is reported through
# the requested completion event rather than blocking process startup.
cat >"$work/invalid-state.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={{type="state/load",namespace="integration",completion="invalid/loaded"}}} end)
  misa.reg_event("invalid/loaded",function(_,event)
    assert(event.ok==false and event.message=="InvalidState")
    return {fx={{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["%s"]}' "$work/invalid-state.lua" >"$work/invalid-state-config.json"
MISA_STATE_FILE="$work/invalid-state.json" MISA_CONFIG="$work/invalid-state-config.json" "$MISA_BIN"

cat >"$work/fake.json" <<'EOF'
{"extensions":["provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["fake response\n"]}}}}
EOF
[ "$(MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello world)" = 'fake response' ]
MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello >"$work/exact-output"
printf 'fake response\n' >"$work/expected-output"
cmp "$work/expected-output" "$work/exact-output"

stage=streaming-lifecycle
cat >"$work/stream-check.lua" <<'LUA'
return {setup=function()
  local lifecycle={start=0,block_start=0,delta=0,block_end=0,finish=0}
  misa.reg_event("transcript/response-start",function() lifecycle.start=lifecycle.start+1 end)
  misa.reg_event("transcript/block-start",function() lifecycle.block_start=lifecycle.block_start+1 end)
  misa.reg_event("transcript/block-delta",function() lifecycle.delta=lifecycle.delta+1 end)
  misa.reg_event("transcript/block-end",function() lifecycle.block_end=lifecycle.block_end+1 end)
  misa.reg_event("transcript/response-end",function() lifecycle.finish=lifecycle.finish+1 end)
  misa.reg_event("agent/completed",function(db)
    assert(db.agent.usage.input_tokens==2 and db.agent.usage.output_tokens==3,"stream usage was not finalized")
    local content=db.agent.messages[2].content
    assert(content[1].type=="text" and content[1].text=="stream ")
    assert(content[2].type=="thinking" and content[2].text=="private")
    assert(content[3].type=="text" and content[3].text=="works")
    assert(lifecycle.start==1 and lifecycle.block_start==3 and lifecycle.delta==3 and lifecycle.block_end==3 and lifecycle.finish==1,"stable response/block lifecycle was not emitted")
    local response=db.messages.responses[2]
    assert(response.status=="complete" and response.block_count==3 and type(response.started_wall_ms)=="number","message response metadata is incomplete")
    assert(db.messages.blocks[2].text=="stream " and db.messages.blocks[2].chunks==nil,"stream chunks were not compacted once")
  end)
end}
LUA
printf '{"extensions":["provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[{"stream":[{"type":"text","text":"stream "},{"type":"thinking","text":"private"},{"type":"text","text":"works"}],"usage":{"input_tokens":2,"output_tokens":3}}]}}}}' "$work/stream-check.lua" >"$work/stream.json"
[ "$(MISA_CONFIG="$work/stream.json" "$MISA_BIN" hello)" = 'stream works' ]

cat >"$work/rate-check.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={
    {type="dispatch",event={type="transcript/response-start",response_id="rate",role="assistant"}},
    {type="dispatch",event={type="transcript/block-start",response_id="rate",block_id="rate/1",kind="assistant"}},
    {type="dispatch",event={type="transcript/block-delta",response_id="rate",block_id="rate/1",text="rated"}},
    {type="timer/start",id="rate",interval_ms=10,completion="rate/finish"},
  }} end)
  misa.reg_event("rate/finish",function() return {fx={{type="timer/stop",id="rate"},{type="dispatch",event={type="transcript/block-end",response_id="rate",block_id="rate/1"}},{type="dispatch",event={type="transcript/response-end",response_id="rate",usage={output_tokens=20}}}}} end)
  misa.reg_event("transcript/response-end",function(db,event)
    if event.response_id~="rate" then return end
    local response=db.messages.responses[1]
    assert(response.output_tokens==20 and response.elapsed_ms>0 and response.tokens_per_second==20000/response.elapsed_ms,"completion rate did not use reported output tokens and monotonic elapsed time")
    return {fx={{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","messages","%s"]}' "$work/rate-check.lua" >"$work/rate-check.json"
[ "$(MISA_CONFIG="$work/rate-check.json" "$MISA_BIN")" = rated ]

cat >"$work/interrupted-check.lua" <<'LUA'
return {setup=function()
  misa.reg_event("agent/completed",function(db)
    assert(#db.agent.messages==1,"interrupted assistant response entered provider history")
    local transcript=db.messages.transcript
    assert(transcript[#transcript-1].interrupted==true and transcript[#transcript-1].text=="partial","partial transcript was not retained")
  end)
end}
LUA
printf '{"extensions":["provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[{"stream":[{"type":"text","text":"partial"}],"error":"stream interrupted"}]}}}}' "$work/interrupted-check.lua" >"$work/interrupted.json"
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
{"extensions":["keybindings","provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","effort","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"models":[{"id":"fake/default","model":"default","api":{"request_options":{"reasoning_effort":{"choices":["low","medium","high"],"default":"low"}}}}],"expect_request_options":{"reasoning_effort":"medium"},"expect_request_options_exact":true,"responses":["cycled"]}}}}
EOF
[ "$(printf '\033ehello\n' | MISA_CONFIG="$work/effort-cycle.json" "$MISA_BIN")" = cycled ]

# A model switch retains equivalent values, but falls back to the new model's
# default when the old value is unavailable.
cat >"$work/effort-fallback.json" <<'EOF'
{"extensions":["provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/wide"},"providers":{"fake":{"models":[{"id":"fake/wide","model":"wide","api":{"request_options":{"reasoning_effort":{"choices":["low","high"],"default":"high"}}}},{"id":"fake/narrow","model":"narrow","api":{"request_options":{"reasoning_effort":{"choices":["low","medium"],"default":"medium"}}}}],"expect_request_options":{"reasoning_effort":"medium"},"expect_request_options_exact":true,"responses":["fallback"]}}}}
EOF
[ "$(printf '/model fake/narrow\nhello\n' | MISA_CONFIG="$work/effort-fallback.json" "$MISA_BIN")" = fallback ]

# Cycling is a no-op for a model without reasoning support.
cat >"$work/effort-unsupported.json" <<'EOF'
{"extensions":["keybindings","provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","effort","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/plain"},"providers":{"fake":{"models":[{"id":"fake/plain","model":"plain"}],"expect_request_options":{},"expect_request_options_exact":true,"responses":["plain"]}}}}
EOF
[ "$(printf '\033ehello\n' | MISA_CONFIG="$work/effort-unsupported.json" "$MISA_BIN")" = plain ]

# Missing generic required metadata blocks provider submission and reports a
# structured request-readiness problem through the harness transcript.
cat >"$work/required-option.json" <<'EOF'
{"extensions":["provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","request_options","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/required"},"providers":{"fake":{"models":[{"id":"fake/required","model":"required","api":{"request_options":{"region":{"required":true,"choices":["east","west"]}}}}],"responses":["must not run"]}}}}
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
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","commands","choice_tree","choices","editor","ui","%s"]}' "$work/command-completion.lua" >"$work/command-completion.json"
[ "$(printf 'discard me\003/p\t\n' | MISA_CONFIG="$work/command-completion.json" "$MISA_BIN")" = pong ]
[ -z "$(printf '\004' | MISA_CONFIG="$work/command-completion.json" "$MISA_BIN")" ]

stage=inline-choices
cat >"$work/inline-choices.lua" <<'LUA'
return {setup=function()
  misa.reg_command({name="/alpha",description="first positional command",event="test/alpha"})
  misa.reg_command({name="/team/one",description="nested one",event="test/team"})
  misa.reg_command({name="/team/two",description="nested two",event="test/team"})
  misa.reg_command({name="/choose",description="inline overlay",event="test/choose",completion="test-values"})
  misa.reg_completion("test-values",{value="alpha"})
  misa.reg_completion("test-values",{value="beta",label="Beta"})
  local function done(text) return {fx={{type="view/commit",lines={{spans={{text=text,style={foreground="default"}}}}}},{type="app/quit"}}} end
  misa.reg_event("test/alpha",function() return done("inline hotkey") end)
  misa.reg_event("test/choose",function(_,event) assert(event.arguments=="beta"); return done("promoted inline") end)
  misa.reg_event("picker/open",function(db,event)
    if event.id=="inline-choice" and event.choose_view then
      assert(db.picker.id=="picker-picker" and db.picker.parent.session==event.session,"inline replace_view did not preserve its session in picker-picker")
      return done("inline view picker")
    end
  end)
  misa.reg_event("terminal/input",function(db,event)
    if event.kind=="text" and event.text=="/" then
      local projection=misa.editor_projection(db); local hinted=false
      for _,line in ipairs(projection.completions) do local text=""; for _,span in ipairs(line.spans) do text=text..span.text end; if text:find("⌥Z",1,true) then hinted=true end end
      assert(hinted,"inline configured positional hint disappeared")
    elseif event.kind=="backspace" and db.editor.text=="/" then
      return done("atomic tree backspace")
    end
  end)
end}
LUA
printf '{"extensions":["fuzzy","keybindings","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","commands","choice_tree","choices","choice_layout","picker","picker_view","editor","ui","%s"],"config":{"choices":{"purposes":{"command-completion":["slash-prefix"]}},"keybindings":{"choices":{"open_overlay":["alt+x"],"option_1_1":["alt+z"]}}}}' "$work/inline-choices.lua" >"$work/inline-choices.json"
[ "$(printf '/\033z\n' | MISA_CONFIG="$work/inline-choices.json" "$MISA_BIN")" = 'inline hotkey' ]
[ "$(printf '/choose \033xb\n\n' | MISA_CONFIG="$work/inline-choices.json" "$MISA_BIN")" = 'promoted inline' ]
[ "$(printf '/choose \033/' | MISA_CONFIG="$work/inline-choices.json" "$MISA_BIN")" = 'inline view picker' ]

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
printf '{"extensions":["keybindings","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","commands","choice_tree","choices","choice_layout","picker","picker_view","editor","ui","%s"]}' "$work/generic-picker.lua" >"$work/generic-picker.json"
[ "$(printf '/choose\nbeta\n' | MISA_CONFIG="$work/generic-picker.json" "$MISA_BIN")" = 'beta/path' ]
# The second visible panel uses the second positional Alt-key bank, but that
# bank cannot activate when terminal geometry hides its panel.
[ "$(printf '/panels\n\033q' | COLUMNS=120 MISA_CONFIG="$work/generic-picker.json" "$MISA_BIN")" = 'beta/path' ]
[ "$(printf '/panels\n\033q\n' | COLUMNS=40 MISA_CONFIG="$work/generic-picker.json" "$MISA_BIN")" = 'alpha' ]
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
printf '{"extensions":["fuzzy","keybindings","commands","choice_tree","choices","preferences","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","choice_layout","picker","picker_view","editor","ui","%s"]}' "$work/generic-command-choice.lua" >"$work/generic-command-choice.json"
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
printf '{"extensions":["%s","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"]}' "$work/dynamic-models.lua" >"$work/dynamic-models.json"
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
printf '{"extensions":["%s","fuzzy","commands","choice_tree","choices","preferences","themes","theme.default","components","layout","choice_layout","picker","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","picker_view","messages","models","agent","editor","ui"],"config":{"models":{"default":"picker/vendor/first"}}}' "$work/model-picker-filter.lua" >"$work/model-picker-filter.json"
# Empty command invocation is intercepted generically, orderless fuzzy search
# highlights independently of the currently selected model, then resumes it.
[ "$(printf '/model\nsecond picker\nhello\n' | MISA_CONFIG="$work/model-picker-filter.json" "$MISA_BIN")" = 'picked vendor/second' ]

stage=unavailable-models
cat >"$work/unavailable-models.lua" <<'LUA'
return {setup=function()
  misa.reg_auth_provider({id="openai",model_provider="private",label="Private",strategy="api_key"})
  misa.reg_model({id="private/model",provider="private",model="model",label="Private model"})
end}
LUA
printf '{"extensions":["%s","auth","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"private/model"}}}' "$work/unavailable-models.lua" >"$work/unavailable-models.json"
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
printf '{"extensions":["json","provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[{"stream":[{"type":"tool_call","index":0,"id":"call-1","name":"echo","arguments_json_delta":"{\\\"value\\\":\\\"from "},{"type":"tool_call","index":0,"arguments_json_delta":"tool\\\"}"}]},"after tool"]}}}}' "$work/tool.lua" >"$work/tool-loop.json"
[ "$(MISA_CONFIG="$work/tool-loop.json" "$MISA_BIN" use tool)" = 'after tool' ]
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"value":"from tool"}}}' |
  MISA_CONFIG="$work/tool-loop.json" "$MISA_BIN" mcp >"$work/mcp-custom-output"
grep -F '"text":"from tool"' "$work/mcp-custom-output" >/dev/null

stage=parallel-tool-order
cat >"$work/parallel-tools.lua" <<'LUA'
return {setup=function()
  for _,name in ipairs({"slow","fast"}) do misa.reg_tool({name=name,description=name.." description",input_schema={type="object"},effect="parallel/run"}) end
  misa.reg_model({id="parallel/model",provider="parallel",model="model"})
  local provider_calls=0
  misa.reg_fx("provider.parallel",function(effect)
    provider_calls=provider_calls+1
    if provider_calls==1 then return {type="dispatch",event={type="agent/result",id=effect.id,content={
      {type="tool_call",id="first",name="slow",arguments={}},{type="tool_call",id="second",name="fast",arguments={}},
    }}} end
    assert(provider_calls==2,"parallel tools continued more than once")
    assert(#effect.messages==4 and effect.messages[3].tool_call_id=="first" and effect.messages[4].tool_call_id=="second","provider history followed completion order instead of assistant call order")
    return {type="dispatch",event={type="agent/result",id=effect.id,content={{type="text",text="parallel ordered"}}}}
  end)
  misa.reg_fx("parallel/run",function(effect) return {type="timer/start",id=effect.tool_call_id,interval_ms=effect.name=="fast" and 10 or 50,completion="parallel/done"} end)
  misa.reg_event("parallel/done",function(_,event) return {fx={{type="timer/stop",id=event.id},{type="dispatch",event={type="tool/result",tool_call_id=event.id,text=event.id}}}} end)
  misa.reg_event("transcript/tool-result",function(db,event)
    if event.id=="second" then
      local first,second=db.messages.blocks[2],db.messages.blocks[3]
      assert(first.call_id=="first" and first.status=="pending" and second.call_id=="second" and second.status=="success","parallel transcript sections did not update independently")
    end
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui","%s"],"config":{"models":{"default":"parallel/model"}}}' "$work/parallel-tools.lua" >"$work/parallel-tools.json"
[ "$(MISA_CONFIG="$work/parallel-tools.json" "$MISA_BIN" parallel)" = 'parallel ordered' ]

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
printf '{"extensions":["json","protocol.openai","protocol.anthropic","%s","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"alpha/model"}}}' "$work/cross-provider.lua" >"$work/cross-provider.json"
[ "$(MISA_CONFIG="$work/cross-provider.json" "$MISA_BIN" replay)" = "portable replay" ]

stage=native-tools
cat >"$work/native-tools.json" <<JSON
{"extensions":["provider.fake","tool.files","tool.shell","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[[{"type":"tool_call","id":"write-1","name":"write_file","arguments":{"path":"$work/native-tool.txt","content":"alpha"}}],[{"type":"tool_call","id":"edit-1","name":"edit_file","arguments":{"path":"$work/native-tool.txt","old_text":"alpha","new_text":"beta"}}],[{"type":"tool_call","id":"read-1","name":"read_file","arguments":{"path":"$work/native-tool.txt"}}],[{"type":"tool_call","id":"list-1","name":"list_directory","arguments":{"path":"$work"}}],[{"type":"tool_call","id":"shell-1","name":"shell","arguments":{"command":"printf shell-ok"}}],"tools done"]}}}}
JSON
[ "$(MISA_CONFIG="$work/native-tools.json" "$MISA_BIN" use native tools)" = 'tools done' ]
[ "$(cat "$work/native-tool.txt")" = beta ]

# Cancelling tool status marks intent, cancels every native call ID, drains all
# completions without another provider request, and leaves visible cancellation.
cat >"$work/cancel-tools.lua" <<'LUA'
return {setup=function()
  local provider_calls,cancel_started=0,false
  for _,name in ipairs({"slow_one","slow_two"}) do
    misa.reg_tool({name=name,description=name,input_schema={type="object",properties={},additionalProperties=false},effect="test/slow"})
  end
  misa.reg_model({id="cancel/model",provider="cancel",model="model"})
  misa.reg_fx("provider.cancel",function(effect)
    provider_calls=provider_calls+1; assert(provider_calls==1,"cancelled tools continued the model request")
    return {type="dispatch",event={type="agent/result",id=effect.id,content={{type="tool_call",id="slow-1",name="slow_one",arguments={}},{type="tool_call",id="slow-2",name="slow_two",arguments={}}}}}
  end)
  misa.reg_fx("test/slow",function(effect) return {type="process/run",id=effect.tool_call_id,argv={"sh","-c","sleep 10"},completion="test/slow-complete"} end)
  misa.reg_event("test/slow-complete",function(_,event) return {fx={{type="dispatch",event={type="tool/result",tool_call_id=event.id,text=event.message or "done",is_error=not event.ok}}}} end)
  misa.reg_event("agent/status",function(_,event)
    if event.status=="tools" and not cancel_started then cancel_started=true; return {fx={{type="timer/start",id="cancel-delay",interval_ms=50,completion="test/cancel"}}} end
  end)
  misa.reg_event("test/cancel",function() return {fx={{type="timer/stop",id="cancel-delay"},{type="dispatch",event={type="agent/cancel-active"}}}} end)
  misa.reg_interceptor({id="test/cancel-effects",after=function(tx)
    if tx.event.type=="agent/cancel-active" then
      assert(tx.db.agent.cancel_requested and tx.db.agent.status=="cancelling","tool cancellation intent was not recorded")
      local ids={}; for _,effect in ipairs(tx.fx) do if effect.type=="operation/cancel" then ids[effect.id]=true end end
      assert(ids["slow-1"] and ids["slow-2"],"not every pending native tool call was cancelled")
    end; return tx
  end})
  misa.reg_event("agent/completed",function(db)
    if not cancel_started then return end
    assert(db.agent.status=="ready" and db.agent.pending_tool_count==0 and not db.agent.cancel_requested,"cancelled tools did not return ready")
    assert(provider_calls==1 and #db.agent.messages==2,"tool completions entered history or continued the model")
    local saw_cancel,saw_interrupted,cancelled_sections=false,false,0
    for _,block in ipairs(db.messages.blocks) do saw_cancel=saw_cancel or (block.kind=="harness" and block.text=="Cancelled"); saw_interrupted=saw_interrupted or block.interrupted==true; if block.kind=="tool_call" and block.status=="cancelled" then cancelled_sections=cancelled_sections+1 end end
    assert(saw_cancel and saw_interrupted and cancelled_sections==2,"cancelled tool-section state was not visible")
    return {fx={{type="view/commit",lines={{spans={{text="cancel tools"}}}}},{type="app/quit"}}}
  end)
end}
LUA
printf '{"extensions":["themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","messages","models","agent","%s"],"config":{"models":{"default":"cancel/model"}}}' "$work/cancel-tools.lua" >"$work/cancel-tools.json"
[ "$(MISA_CONFIG="$work/cancel-tools.json" "$MISA_BIN" cancel)" = "$(printf 'Cancelled\ncancel tools')" ]

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
{"extensions":["ui","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","agent","models","commands","choice_tree","choices","editor","provider.fake"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["ui first"]}}}}
EOF
[ "$(MISA_CONFIG="$work/ui-first.json" "$MISA_BIN" hello)" = 'ui first' ]

# Provider-originated message strings are normalized at the transcript boundary,
# even when they did not pass through the process-output sanitizer.
cat >"$work/message-controls.json" <<'EOF'
{"extensions":["provider.fake","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["ok\tred\u001b[31m!\u001b[0m\u0001\u0085"]}}}}
EOF
[ "$(MISA_CONFIG="$work/message-controls.json" "$MISA_BIN" hello)" = 'ok red!' ]

# The standard editor inserts at its UTF-8 byte cursor, but movement and
# deletion use layout/presenter grapheme boundaries. The provider echoes text.
cat >"$work/echo-prompt" <<'SH'
#!/bin/sh
for prompt do :; done
printf '%s\n' "$prompt"
SH
chmod +x "$work/echo-prompt"
printf '{"extensions":["provider.command","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"command/default"},"providers":{"command":{"argv":["%s"]}}}}' "$work/echo-prompt" >"$work/editor.json"
printf 'ac\033[Db\033[D\033[Cd\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'abdc\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'aéx\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'aéx\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'a👩‍💻x\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'aक्x\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'aक्षx\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'क्षx\033[D\033[D\033[CZ\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'क्षZx\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'éx\033[D\033[D\033[CZ\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'éZx\n' >"$work/expected-editor-output"
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
{"extensions":["provider.command","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"command/default"},"providers":{"command":{"argv":["$work/provider","fixed;word"]}}}}
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
# A semantic result must make Misa kill us rather than waiting for process EOF.
sleep 3
SH
chmod +x "$work/claude"
printf '{"extensions":["provider.claude","tool.files","themes","theme.default","components","layout","markdown","component.markdown","component.tool","component.message","component.editor","component.picker","component.status","component.chrome","messages","models","agent","commands","choice_tree","choices","editor","ui"],"config":{"models":{"default":"claude/claude-sonnet-5"},"providers":{"claude":{"max_plan":true,"executable":"%s","mcp_command":"%s","mcp_arguments":["mcp","--config","%s"]}}}}' "$work/claude" "$MISA_BIN" "$work/claude.json" >"$work/claude.json"
claude_started=$(date +%s)
claude_output="$(MISA_CONFIG="$work/claude.json" "$MISA_BIN" hello)"
claude_elapsed=$(($(date +%s) - claude_started))
if [ "$claude_elapsed" -ge 2 ]; then echo "claude waited for process EOF" >&2; exit 1; fi
if [ "$claude_output" != 'claude result' ]; then echo "claude output: $claude_output" >&2; exit 1; fi

stage=dialog-lifecycle
cat >"$work/dialog-lifecycle.lua" <<'LUA'
return {setup=function()
  misa.reg_event("app/start",function() return {fx={{type="dispatch",event={type="dialog/open",id="one",correlation="a",completion="dialog/done",kind="progress",title="Work",message="waiting",cancellable=true}}}} end)
  misa.reg_event("dialog/done",function(db,event)
    if event.correlation=="a" then
      assert(event.cancelled and db.dialog==nil,"cancel did not close its dialog")
      return {db=db,fx={{type="dispatch",event={type="dialog/open",id="two",correlation="b",completion="dialog/done",kind="modal",title="Input",message="paste",input=true,actions={{id="submit",label="submit"}}}},{type="dispatch",event={type="dialog/input",kind="text",text="code"}},{type="dispatch",event={type="dialog/input",kind="enter"}}}}
    end
    assert(not event.cancelled and event.value=="code" and event.action=="submit" and db.dialog==nil,"submit lifecycle failed")
    return {fx={{type="view/commit",lines={{spans={{text="dialogs",style={foreground="default"}}}}}},{type="app/quit"}}}
  end)
  misa.reg_event("dialog/opened-test",function() end)
  misa.reg_event("dialog/begin-cancel",function() return {fx={{type="dispatch",event={type="dialog/input",kind="escape"}}}} end)
  misa.reg_interceptor({id="dialog-test-cancel",after=function(tx) if tx.event.type=="dialog/open" and tx.event.id=="one" then tx.fx[#tx.fx+1]={type="dispatch",event={type="dialog/update",id="one",correlation="stale",message="bad"}}; tx.fx[#tx.fx+1]={type="dispatch",event={type="dialog/begin-cancel"}} end return tx end})
end}
LUA
printf '{"extensions":["dialogs","%s"]}' "$work/dialog-lifecycle.lua" >"$work/dialog-lifecycle.json"
[ "$(MISA_CONFIG="$work/dialog-lifecycle.json" "$MISA_BIN")" = dialogs ]

stage=contract-errors
# Config and argv remain available in base cofx; plain output contains no ANSI.
cat >"$work/context.lua" <<'LUA'
return {setup=function(context)
  local ok = pcall(function() misa.reg_cofx("terminal", function() end) end)
  assert(not ok, "terminal cofx name must be reserved")
  misa.reg_event("app/start",function(db,event,cofx)
    assert(cofx.config.value==42 and cofx.argv[1]=='arg')
    assert(cofx.terminal.interactive == false and cofx.terminal.columns == 37 and cofx.terminal.lines == 11)
    return {fx={{type="view/commit",lines={{spans={{text="plain",style={foreground="cyan"}}}}}},{type="app/quit"}}}
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
