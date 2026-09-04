#!/bin/sh
set -eu
MISA_BIN="$1"

unset MISA_CONFIG MISA_EXTENSION_DIR
# A closed non-TTY stdin is an EOF input event, not a session failure.
[ -z "$("$MISA_BIN" </dev/null)" ]
actual="$("$MISA_BIN" smoke)"
[ "$actual" = 'misa is running' ] || {
  printf 'installed default config/extension lookup failed: %s\n' "$actual" >&2
  exit 1
}
