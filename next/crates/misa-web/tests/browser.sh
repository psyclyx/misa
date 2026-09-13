#!/usr/bin/env bash
set -euo pipefail
browser_root=$(cd "$(dirname "$0")" && pwd)
browser_profile=$(mktemp -d)
trap 'rm -rf "$browser_profile"' EXIT
export XDG_CONFIG_HOME="$browser_profile/config"
export XDG_CACHE_HOME="$browser_profile/cache"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"
if ! chromium --headless --no-sandbox --disable-gpu --disable-dev-shm-usage \
  --allow-file-access-from-files --user-data-dir="$browser_profile" \
  --dump-dom --virtual-time-budget=3000 "file://$browser_root/browser.html" > "$browser_profile/result.html" 2> "$browser_profile/browser.log"; then
  cat "$browser_profile/browser.log"
  exit 1
fi
if ! grep -q 'data-result="passed"' "$browser_profile/result.html"; then
  cat "$browser_profile/result.html" "$browser_profile/browser.log"
  exit 1
fi
printf '%s\n' 'Browser memory checks passed'
