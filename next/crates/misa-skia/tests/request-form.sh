#!/usr/bin/env bash
# Run from next/ with freshly built misa-skia and misa-daemon, Xvfb, xdotool, xclip.
# All daemons, identities, runtime discovery sockets, and preferences are isolated.
set -euo pipefail
binary=${1:-${CARGO_TARGET_DIR:-target}/debug/misa-skia}
daemon_binary=${2:-${CARGO_TARGET_DIR:-target}/debug/misa-daemon}
component=${3:?Pass the built pet component as the third argument}
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
"$daemon_binary" --no-relay --plugin "$component" --session same-label </dev/null >"$work/daemon1.log" 2>&1 &
daemon1=$!
"$daemon_binary" --no-relay --plugin "$component" --session same-label </dev/null >"$work/daemon2.log" 2>&1 &
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
click 80 286
key ctrl+shift+i
click 80 309 # pet portable variant
key Escape ctrl+i
click 80 596 # Choose treats, installed binding
key ctrl+r # explicit local request visibility
type bad
key Return
sleep .5
test "$(copy_text)" = bad
cp "$work/frame.png" "$work/invalid-request-retains-draft.png"
key Escape ctrl+r
test "$(copy_text)" = bad
type 7
key Return
sleep 1
cp "$work/frame.png" "$work/resolved-pet.png"
click 80 596
key ctrl+r
type invalid
click 80 166 # cancellation does not parse invalid draft
sleep 1
cp "$work/frame.png" "$work/cancelled-request.png"
echo "Native custom request validation, retained drafts, resolution and cancellation passed; artifacts: $work"
