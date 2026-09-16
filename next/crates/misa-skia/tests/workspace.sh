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
"$daemon_binary" --no-relay --tool-approval ask --session same-label </dev/null >"$work/daemon1.log" 2>&1 &
daemon1=$!
"$daemon_binary" --no-relay --tool-approval ask --session same-label </dev/null >"$work/daemon2.log" 2>&1 &
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
cp "$work/frame.png" "$work/discovered.png"
click 80 286
type 'draft on first daemon'
key ctrl+o
click 80 464
type 'draft on second daemon'
key ctrl+o
click 80 286
test "$(copy_text)" = 'draft on first daemon'
key ctrl+o
click 80 369 # disconnect first relationship, preserve its local instance
click 80 464 # retained first instance after the connected daemon
test "$(copy_text)" = 'draft on first daemon'
key ctrl+o
cp "$work/frame.png" "$work/disconnected-retained.png"
click 80 286 # healthy second daemon
test "$(copy_text)" = 'draft on second daemon'
key ctrl+a
type '/status'
key Return
sleep .5
key ctrl+c
timeout 5 xclip -selection clipboard -o >"$work/healthy-daemon-status.txt"
rg -q 'provider: scripted' "$work/healthy-daemon-status.txt"
key Escape
type '/login anthropic'
key Return
sleep .5
key ctrl+r
cp "$work/frame.png" "$work/credential-open.png"
type 'dummy-local-secret'
cp "$work/frame.png" "$work/credential-masked.png"
key Escape ctrl+r
cp "$work/frame.png" "$work/credential-cleared.png"
key ctrl+c
test "$(timeout 5 xclip -selection clipboard -o)" = "$(cat "$work/healthy-daemon-status.txt")"
click 80 167 # explicit credential operation cancellation
type 'exercise tool approval'
key Return
sleep 1
key ctrl+r
cp "$work/frame.png" "$work/tool-request.png"
click 80 204 # approve echo tool
sleep 1
cp "$work/frame.png" "$work/tool-completed.png"
type '/status'
key Return
sleep .5
key ctrl+c
timeout 5 xclip -selection clipboard -o >"$work/completed-status.txt"
rg -q 'status: idle' "$work/completed-status.txt"
key Escape ctrl+shift+i
click 80 84 # hide status presentation before replacement subscription
key Escape
cp "$work/frame.png" "$work/status-hidden.png"
kill -0 "$app" "$daemon1" "$daemon2"
printf 'native discovery, isolated drafts, disconnect, private credentials, tool workflow and composition passed\nArtifacts: %s\n' "$work"
