#!/bin/sh
set -eu
MISA_BIN="$1"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat >"$work/config.json" <<'EOF'
{"extensions":["provider.fake","agent"],"config":{"agent":{"provider":"fake"},"providers":{"fake":{"responses":["installed lookup ok"]}}}}
EOF

unset MISA_EXTENSION_DIR
actual="$(MISA_CONFIG="$work/config.json" "$MISA_BIN" smoke)"
[ "$actual" = 'installed lookup ok' ] || {
  printf 'installed-layout lookup failed: %s\n' "$actual" >&2
  exit 1
}
