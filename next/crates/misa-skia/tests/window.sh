#!/usr/bin/env bash
# Run from next/ in the artifact shell with a built pixel executable.
set -euo pipefail
binary=${1:-${CARGO_TARGET_DIR:-target}/debug/misa-skia}
work=$(mktemp -d)
export DISPLAY=:${MISA_TEST_DISPLAY:-193}
Xvfb "$DISPLAY" -screen 0 1024x768x24 >"$work/xvfb.log" 2>&1 &
xvfb=$!
trap 'kill "$xvfb" ${app:-} 2>/dev/null || true; rm -rf "$work"' EXIT
sleep 1
"$binary" --view crates/misa-skia/tests/window.json --window --out "$work/frame.png" >"$work/window.log" 2>&1 &
app=$!
window=$(timeout 20 xdotool search --sync --name 'pixels' | head -1)
test -n "$window"
xdotool windowfocus "$window"
xdotool mousemove --window "$window" 80 55 click 1
xdotool type --window "$window" --clearmodifiers 'window input works'
xdotool key --window "$window" ctrl+a ctrl+c
sleep 1
test "$(timeout 5 xclip -selection clipboard -o)" = 'window input works'
cp "$work/frame.png" "$work/before.png"
xdotool mousemove --window "$window" 80 102 click 1
sleep 1
! cmp -s "$work/before.png" "$work/frame.png"
printf 'native window keyboard, clipboard, disclosure redraw passed\n'
