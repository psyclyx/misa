#!/usr/bin/env bash
# Run from next/ with freshly built misa-skia and misa-daemon, Xvfb, xdotool, xclip.
# All daemons, identities, runtime discovery sockets, and preferences are isolated.
set -euo pipefail
binary=${1:-${CARGO_TARGET_DIR:-target}/debug/misa-skia}
daemon_binary=${2:-${CARGO_TARGET_DIR:-target}/debug/misa-daemon}
work=$(mktemp -d /tmp/misa-native-workspace-test-XXXXXX)
export XDG_RUNTIME_DIR="$work/runtime" XDG_STATE_HOME="$work/state" MISA_PREFS="$work/prefs.json"
mkdir -m 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY WAYLAND_SOCKET
cleanup() { kill ${app:-} ${daemon1:-} ${daemon2:-} ${xvfb:-} 2>/dev/null || true; wait ${app:-} ${daemon1:-} ${daemon2:-} ${xvfb:-} 2>/dev/null || true; }
trap cleanup EXIT
Xvfb -displayfd 3 -screen 0 1280x960x24 3>"$work/display" >"$work/xvfb.log" 2>&1 &
xvfb=$!
for _ in $(seq 1 100); do test -s "$work/display" && break; sleep .05; done
export DISPLAY=:$(cat "$work/display")
"$daemon_binary" --no-relay --session same-label </dev/null >"$work/daemon1.log" 2>&1 &
daemon1=$!
"$daemon_binary" --no-relay --session same-label </dev/null >"$work/daemon2.log" 2>&1 &
daemon2=$!
for _ in $(seq 1 200); do rg -q '^misa:' "$work/daemon1.log" && rg -q '^misa:' "$work/daemon2.log" && break; sleep .05; done
"$binary" --window --out "$work/frame.png" >"$work/window.log" 2>&1 &
app=$!
window=$(timeout 20 xdotool search --sync --name pixels | head -1)
xdotool windowfocus "$window"
sleep 1
click() { xdotool mousemove --window "$window" "$1" "$2" click 1; sleep .4; }
key() { xdotool key --window "$window" "$@"; sleep .3; }
type() { xdotool type --window "$window" --clearmodifiers "$1"; sleep .2; }
copy_text() { key ctrl+a ctrl+c; timeout 5 xclip -selection clipboard -o; }
# Open the first daemon's management surface, then installed create form.
click 80 513
cp "$work/frame.png" "$work/overview.png"
click 80 315
click 80 80
type 'native-created'
click 80 311
sleep 1
type 'native lifecycle history'
key Return
sleep 2
type 'retained draft'
key ctrl+o
# The directory gained a session and a retained local-instance control.
xdotool mousemove --window "$window" 850 20 click --repeat 12 --delay 20 4
sleep .3
click 80 625
cp "$work/frame.png" "$work/working-overview.png"
click 80 125 # stop native-created on this daemon only
sleep 1
cp "$work/frame.png" "$work/stopped-retained.png"
click 80 194 # reopen retained local instance
 test "$(copy_text)" = 'retained draft'
key ctrl+o
xdotool mousemove --window "$window" 850 20 click --repeat 12 --delay 20 4
sleep .3
click 80 625
click 80 297 # finite archived conversation search
sleep 1
cp "$work/frame.png" "$work/archive.png"
click 80 365 # resume selected stored conversation
cp "$work/frame.png" "$work/resume-form.png"
click 80 145
type 'native-resumed'
click 80 375
sleep 1
cp "$work/frame.png" "$work/resumed.png"
echo "Native lifecycle UI gate passed; inspect overview/archive/resumed artifacts: $work"
