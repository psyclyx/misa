#!/usr/bin/env bash
# Native data-control clipboard test; deliberately cannot use X11/XWayland.
set -euo pipefail
work=$(mktemp -d)
export XDG_RUNTIME_DIR="$work/runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"
unset DISPLAY WAYLAND_DISPLAY SWAYSOCK
export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
cat > "$work/sway.conf" <<'CONFIG'
xwayland disable
seat seat0 fallback true
output HEADLESS-1 resolution 800x600
CONFIG
sway --config "$work/sway.conf" > "$work/sway.log" 2>&1 &
compositor=$!
trap 'kill "$compositor" 2>/dev/null || true; wait "$compositor" 2>/dev/null || true; rm -rf "$work"' EXIT
for attempt in $(seq 1 100); do
  for socket in "$XDG_RUNTIME_DIR"/wayland-*; do
    if test -S "$socket"; then export WAYLAND_DISPLAY="${socket##*/}"; break 2; fi
  done
  if ! kill -0 "$compositor" 2>/dev/null; then cat "$work/sway.log"; exit 1; fi
  sleep 0.1
done
if test -z "${WAYLAND_DISPLAY:-}"; then cat "$work/sway.log"; exit 1; fi
unset LD_LIBRARY_PATH
export MISA_WAYLAND_CLIPBOARD_TEST=1
timeout 120 cargo test "$@" -p misa-tui desktop_image_and_text_roundtrip -- --ignored --nocapture
printf 'native Wayland text and PNG clipboard roundtrip passed (DISPLAY unset)\n'
