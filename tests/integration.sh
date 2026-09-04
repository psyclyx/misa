#!/bin/sh
set -eu
MISA_BIN="$1"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cat >"$work/first.lua" <<'LUA'
return {
  setup = function(context)
    io.write("setup:first\n")
    misa.register("decorate", function(value) return "first:" .. value end)
    misa.register("maybe", function() return nil end)
  end,
  run = function(context)
    io.write("run:first:" .. context.config_json .. ":" .. table.concat(context.argv, ",") .. "\n")
  end,
}
LUA
cat >"$work/second.lua" <<'LUA'
return {
  setup = function(context)
    io.write("setup:second\n")
    misa.register("decorate", function(value) return "second:" .. value end)
    misa.register("maybe", function() return "present" end)
  end,
  run = function(context)
    local values = context.misa.call("decorate", "ok")
    local maybe = context.misa.call("maybe")
    assert(maybe.n == 2 and maybe[1] == nil and maybe[2] == "present")
    io.write("run:second:" .. values.n .. ":" .. table.concat(values, ",") .. "\n")
  end,
}
LUA
cat >"$work/config.json" <<EOF
{"extensions":["$work/first.lua","$work/second.lua"],"config":{"visible":true}}
EOF

actual="$($MISA_BIN forwarded-a --config "$work/config.json" forwarded-b -- --config opaque)"
expected='setup:first
setup:second
run:first:{"visible":true}:forwarded-a,forwarded-b,--config,opaque
run:second:2:first:ok,second:ok'
[ "$actual" = "$expected" ] || {
  printf 'unexpected output:\n%s\n' "$actual" >&2
  exit 1
}

actual="$(MISA_CONFIG="$work/config.json" "$MISA_BIN" fallback -- --config forwarded)"
printf '%s\n' "$actual" | grep -F 'run:first:{"visible":true}:fallback,--config,forwarded' >/dev/null

if "$MISA_BIN" --config "$work/config.json" --config "$work/config.json" 2>"$work/error"; then
  echo 'duplicate config unexpectedly succeeded' >&2
  exit 1
fi
grep -F -- '--config may only be specified once' "$work/error" >/dev/null

printf '{}' >"$work/defaults.json"
[ -z "$(MISA_CONFIG="$work/defaults.json" "$MISA_BIN")" ]

cat >"$work/decoded.lua" <<'LUA'
return { run = function(context)
  assert(context.config.object.string == "value")
  assert(context.config.object.boolean == true)
  assert(context.config.integer == 42 and context.config.float == 1.5)
  assert(context.config.array[1] == "first")
  assert(rawequal(context.config.null_value, misa.json_null))
  assert(rawequal(context.config.array[2], context.config.null_value))
  assert(context.config_json == '{"object":{"string":"value","boolean":true},"integer":42,"float":1.5,"array":["first",null],"null_value":null}')
  io.write("decoded config ok\n")
end }
LUA
cat >"$work/decoded.json" <<EOF
{"extensions":["$work/decoded.lua"],"config":{"object":{"string":"value","boolean":true},"integer":42,"float":1.5,"array":["first",null],"null_value":null}}
EOF
[ "$(MISA_CONFIG="$work/decoded.json" "$MISA_BIN")" = 'decoded config ok' ]

cat >"$work/fake.json" <<'EOF'
{"extensions":["provider.fake","agent"],"config":{"agent":{"provider":"fake","system_prompt":"be brief"},"providers":{"fake":{"responses":["fake response\n"]}}}}
EOF
[ "$(MISA_CONFIG="$work/fake.json" "$MISA_BIN" hello world)" = 'fake response' ]
# Existing line endings are preserved, while missing ones are supplied once.
printf '%s' '{"extensions":["provider.fake","agent"],"config":{"agent":{"provider":"fake"},"providers":{"fake":{"responses":["x\n"]}}}}' >"$work/newline.json"
MISA_CONFIG="$work/newline.json" "$MISA_BIN" prompt >"$work/output"
[ "$(wc -c <"$work/output")" -eq 2 ]
printf '%s' '{"extensions":["provider.fake","agent"],"config":{"agent":{"provider":"fake"},"providers":{"fake":{"responses":["x"]}}}}' >"$work/newline.json"
MISA_CONFIG="$work/newline.json" "$MISA_BIN" prompt >"$work/output"
[ "$(wc -c <"$work/output")" -eq 2 ]

cat >"$work/mixed-first.lua" <<'LUA'
return { setup = function() io.write("setup:first-custom\n") end, run = function() io.write("run:first-custom\n") end }
LUA
cat >"$work/mixed-last.lua" <<'LUA'
return { setup = function() io.write("setup:last-custom\n") end, run = function() io.write("run:last-custom\n") end }
LUA
cat >"$work/mixed.json" <<EOF
{"extensions":["$work/mixed-first.lua","provider.fake","$work/mixed-last.lua"],"config":{"providers":{"fake":{"responses":[]}}}}
EOF
actual="$(MISA_CONFIG="$work/mixed.json" "$MISA_BIN")"
expected='setup:first-custom
setup:last-custom
run:first-custom
run:last-custom'
[ "$actual" = "$expected" ]

printf '{"extensions":["provider.unknown"]}' >"$work/unknown.json"
if MISA_CONFIG="$work/unknown.json" "$MISA_BIN" 2>"$work/error"; then
  echo 'unknown standard extension unexpectedly succeeded' >&2
  exit 1
fi
grep -F "unknown standard extension ID 'provider.unknown'" "$work/error" >/dev/null
grep -F "path containing '/' or ending in .lua" "$work/error" >/dev/null

printf '%s' '{"extensions":["bad\u0000path.lua"]}' >"$work/nul-extension.json"
if MISA_CONFIG="$work/nul-extension.json" "$MISA_BIN" 2>"$work/error"; then
  echo 'NUL extension path unexpectedly succeeded' >&2
  exit 1
fi
grep -F 'ExtensionContainsNul' "$work/error" >/dev/null

# 128 nested containers are accepted; the next level is a configuration error.
awk 'BEGIN { printf "{\"config\":"; for (i=0;i<128;i++) printf "["; printf "0"; for (i=0;i<128;i++) printf "]"; printf "}" }' >"$work/deep-ok.json"
[ -z "$(MISA_CONFIG="$work/deep-ok.json" "$MISA_BIN")" ]
awk 'BEGIN { printf "{\"config\":"; for (i=0;i<129;i++) printf "["; printf "0"; for (i=0;i<129;i++) printf "]"; printf "}" }' >"$work/deep.json"
if MISA_CONFIG="$work/deep.json" "$MISA_BIN" 2>"$work/error"; then
  echo 'overly deep config unexpectedly succeeded' >&2
  exit 1
fi
grep -F 'nesting exceeds maximum depth of 128' "$work/error" >/dev/null

cat >"$work/provider-command" <<'SH'
#!/bin/sh
[ "$#" -eq 2 ] || exit 20
[ "$1" = "single'quote" ] || exit 21
[ "$2" = "it's safe" ] || exit 22
printf 'command response: %s\n' "$2"
SH
chmod +x "$work/provider-command"
cat >"$work/command.json" <<EOF
{"extensions":["provider.command","agent"],"config":{"agent":{"provider":"command"},"providers":{"command":{"argv":["$work/provider-command","single'quote"]}}}}
EOF
[ "$(MISA_CONFIG="$work/command.json" "$MISA_BIN" "it's" safe)" = "command response: it's safe" ]
printf '%s' '{"extensions":["provider.command"],"config":{"providers":{"command":{"argv":["echo","bad\u0000arg"]}}}}' >"$work/command-nul.json"
if MISA_CONFIG="$work/command-nul.json" "$MISA_BIN" 2>"$work/error"; then
  echo 'NUL command argument unexpectedly succeeded' >&2
  exit 1
fi
grep -F 'argv must not contain NUL' "$work/error" >/dev/null

cat >"$work/metatable.lua" <<'LUA'
local extension = {
  run = function() io.write("raw lookup survived\n") end,
}
return setmetatable(extension, {
  __index = function() error("metatable lookup must not run") end,
})
LUA
printf '{"extensions":["%s"]}' "$work/metatable.lua" >"$work/metatable.json"
[ "$(MISA_CONFIG="$work/metatable.json" "$MISA_BIN")" = 'raw lookup survived' ]

cat >"$work/malformed.lua" <<'LUA'
return { setup = true }
LUA
printf '{"extensions":["%s"]}' "$work/malformed.lua" >"$work/malformed.json"
if MISA_CONFIG="$work/malformed.json" "$MISA_BIN" 2>"$work/error"; then
  echo 'malformed callback unexpectedly succeeded' >&2
  exit 1
fi
grep -F "$work/malformed.lua" "$work/error" >/dev/null
grep -F "field 'setup' must be a function" "$work/error" >/dev/null

cat >"$work/failing.lua" <<'LUA'
local function nested_failure()
  error("callback exploded")
end
return { run = function() nested_failure() end }
LUA
printf '{"extensions":["%s"]}' "$work/failing.lua" >"$work/failing.json"
if MISA_CONFIG="$work/failing.json" "$MISA_BIN" 2>"$work/error"; then
  echo 'failing callback unexpectedly succeeded' >&2
  exit 1
fi
grep -F "$work/failing.lua" "$work/error" >/dev/null
grep -F 'callback exploded' "$work/error" >/dev/null
grep -F 'stack traceback:' "$work/error" >/dev/null

if "$MISA_BIN" 2>"$work/error"; then
  echo 'missing config unexpectedly succeeded' >&2
  exit 1
fi
grep -F 'pass --config PATH or set MISA_CONFIG' "$work/error" >/dev/null
