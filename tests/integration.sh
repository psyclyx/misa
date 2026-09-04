#!/bin/sh
set -eu
case "$1" in /*) MISA_BIN="$1" ;; *) MISA_BIN="$PWD/${1#./}" ;; esac
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
export MISA_AUTH_FILE="$work/session-auth.json"

# No extensions remains completely silent.
printf '{}' >"$work/empty.json"
[ -z "$(MISA_AUTH_FILE="$work/empty-auth.json" MISA_CONFIG="$work/empty.json" "$MISA_BIN")" ]

# Explicit API-key login writes only the XDG credential store with restrictive
# permissions; secrets never enter the ordinary config.
printf 'test-secret\n' | env -u MISA_AUTH_FILE XDG_STATE_HOME="$work/state" "$MISA_BIN" login openai 2>"$work/login-output"
grep -F 'saved openai credential' "$work/login-output" >/dev/null
! grep -F 'test-secret' "$work/login-output" >/dev/null
[ "$(stat -c %a "$work/state/misa")" = 700 ]
[ "$(stat -c %a "$work/state/misa/auth.json")" = 600 ]
grep -F 'test-secret' "$work/state/misa/auth.json" >/dev/null
mkdir "$work/bin"
cat >"$work/bin/claude" <<'SH'
#!/bin/sh
[ "$1 $2" = "auth login" ]
SH
chmod +x "$work/bin/claude"
PATH="$work/bin:$PATH" "$MISA_BIN" login claude
[ ! -e "$work/session-auth.json" ]

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

cat >"$work/fake.json" <<'EOF'
{"extensions":["provider.fake","models","agent","ui"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["fake response\n"]}}}}
EOF
[ "$(MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello world)" = 'fake response' ]
MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello >"$work/exact-output"
printf 'fake response\n' >"$work/expected-output"
cmp "$work/expected-output" "$work/exact-output"

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
printf '{"extensions":["provider.fake","models","agent","ui","%s"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":[[{"type":"tool_call","id":"call-1","name":"echo","arguments_json":"{\\\"value\\\":\\\"from tool\\\"}"}],"after tool"]}}}}' "$work/tool.lua" >"$work/tool-loop.json"
[ "$(MISA_CONFIG="$work/tool-loop.json" "$MISA_BIN" use tool)" = 'after tool' ]

# Completion does not depend on ui being registered after agent/provider.
cat >"$work/ui-first.json" <<'EOF'
{"extensions":["ui","agent","models","provider.fake"],"config":{"models":{"default":"fake/default"},"providers":{"fake":{"responses":["ui first"]}}}}
EOF
[ "$(MISA_CONFIG="$work/ui-first.json" "$MISA_BIN" hello)" = 'ui first' ]

# The standard editor inserts at its UTF-8 byte cursor, moves both directions,
# and backspaces one complete codepoint. The provider echoes the submitted text.
cat >"$work/echo-prompt" <<'SH'
#!/bin/sh
for prompt do :; done
printf '%s\n' "$prompt"
SH
chmod +x "$work/echo-prompt"
printf '{"extensions":["provider.command","models","agent","ui"],"config":{"models":{"default":"command/default"},"providers":{"command":{"argv":["%s"]}}}}' "$work/echo-prompt" >"$work/editor.json"
printf 'ac\033[Db\033[D\033[Cd\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'abdc\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf 'aéx\033[D\177\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ax\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"
printf '\033[200~ab\ncd\033[201~\033[D\177X\n' | MISA_CONFIG="$work/editor.json" "$MISA_BIN" >"$work/editor-output"
printf 'ab\nXd\n' >"$work/expected-editor-output"
cmp "$work/expected-editor-output" "$work/editor-output"

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
{"extensions":["provider.command","models","agent","ui"],"config":{"models":{"default":"command/default"},"providers":{"command":{"argv":["$work/provider","fixed;word"]}}}}
EOF
(cd "$work" && MISA_CONFIG="$work/command.json" "$MISA_BIN" '$(touch SHOULD_NOT_EXIST);' "it's" literal >"$work/command-output" && [ ! -e SHOULD_NOT_EXIST ])
printf 'result $(touch SHOULD_NOT_EXIST); it'"'"'s literal:�\n' >"$work/expected-command-output"
cmp "$work/expected-command-output" "$work/command-output"

# Claude provider speaks the CLI's stream-json protocol, including the empty
# --tools argument, without requiring the Agent SDK package or copying auth.
cat >"$work/claude" <<'SH'
#!/bin/sh
saw_empty=false
for arg do [ -z "$arg" ] && saw_empty=true; done
[ "$saw_empty" = true ] || exit 30
printf '%s\n' '{"type":"system","subtype":"init","session_id":"test"}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"claude result","session_id":"test"}'
SH
chmod +x "$work/claude"
printf '{"extensions":["provider.claude","models","agent","ui"],"config":{"models":{"default":"claude/sonnet"},"providers":{"claude":{"executable":"%s"}}}}' "$work/claude" >"$work/claude.json"
[ "$(MISA_CONFIG="$work/claude.json" "$MISA_BIN" hello)" = 'claude result' ]

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
grep -E 'nesting|depth' "$work/error" >/dev/null
