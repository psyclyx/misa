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
sleep 2
cp "$work/frame.png" "$work/initial.png"
click 80 286
sleep 1
key ctrl+shift+p
cp "$work/frame.png" "$work/installed-commands.png"
key Escape ctrl+shift+i
click 80 398 # local light theme
sleep .5
cp "$work/frame.png" "$work/light.png"
for _ in $(seq 1 30); do test "$(cat "$XDG_STATE_HOME/misa/identities/misa-skia-appearance" 2>/dev/null)" = light && break; sleep .1; done
test "$(cat "$XDG_STATE_HOME/misa/identities/misa-skia-appearance")" = light
kill "$app"; wait "$app" || true
"$binary" --window --out "$work/reopened.png" >"$work/reopened.log" 2>&1 &
app=$!
window=$(timeout 20 xdotool search --sync --name pixels | head -1)
xdotool windowfocus "$window"
sleep 1
cp "$work/reopened.png" "$work/persisted-light.png"
node - "$work/light.png" "$work/persisted-light.png" <<'NODE'
const fs=require('fs'),zlib=require('zlib');
for(const path of process.argv.slice(2)) {const b=fs.readFileSync(path),chunks=[];let at=8;while(at<b.length){const n=b.readUInt32BE(at),id=b.toString('ascii',at+4,at+8);if(id==='IDAT')chunks.push(b.subarray(at+8,at+8+n));at+=n+12;}const raw=zlib.inflateSync(Buffer.concat(chunks));if(raw[1]<230||raw[2]<230||raw[3]<230)throw Error('Light theme did not reach pixels: '+path);}
NODE
echo "Native installed command discovery and persisted light theme passed; artifacts: $work"
