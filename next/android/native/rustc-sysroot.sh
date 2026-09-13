#!/usr/bin/env bash
set -euo pipefail
exec "$ANDROID_REAL_RUSTC" --sysroot "$ANDROID_BUILD_SYSROOT" "$@"
