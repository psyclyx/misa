#!/bin/sh
set -eu
MISA_BIN="$1"

unset MISA_CONFIG MISA_EXTENSION_DIR
# The installed profile is a real Claude coding-agent setup. Loading it does
# not contact Claude until a prompt is submitted.
[ "$("$MISA_BIN" </dev/null)" = 'misa> enter a prompt:' ]
