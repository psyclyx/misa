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
MISA_CONFIG="$work/defaults.json" "$MISA_BIN"

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
