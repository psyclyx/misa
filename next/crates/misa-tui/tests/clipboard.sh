#!/usr/bin/env bash
# Run from next/ in the artifact shell.
set -euo pipefail
export DISPLAY=:${MISA_CLIPBOARD_DISPLAY:-194}
clipboard_log=$(mktemp)
Xvfb "$DISPLAY" -screen 0 1024x768x24 >"$clipboard_log" 2>&1 &
clipboard_display=$!
trap 'kill "$clipboard_display" 2>/dev/null || true; rm -f "$clipboard_log"' EXIT
sleep 1
cargo test "$@" -p misa-tui desktop_image_and_text_roundtrip -- --ignored
