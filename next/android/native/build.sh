#!/usr/bin/env bash
# Explicit native build, separate from Gradle. Run from the pinned project shell.
#
# The Android library is the *client* half of misa: the same `misa-net` transport
# and `misa-client` kit the terminal, browser, and pixel frontends use. That is
# what makes the phone a frontend rather than a second implementation.
set -euo pipefail
: "${ANDROID_NDK_HOME:?Set installed Android NDK path}"
android_native=$(cd -- "$(dirname -- "$0")" && pwd)
android_target=${1:-x86_64-linux-android}
case "$android_target" in
  x86_64-linux-android | aarch64-linux-android | armv7-linux-androideabi) ;;
  *)
    echo "unsupported android target: $android_target" >&2
    exit 2
    ;;
esac
android_profile=${ANDROID_NATIVE_PROFILE:-debug}
android_profile_flags=()
case "$android_profile" in
  debug) ;;
  release)
    android_profile_flags=(--release)
    export CARGO_PROFILE_RELEASE_LTO=thin
    export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
    ;;
  *)
    echo 'ANDROID_NATIVE_PROFILE must be debug or release' >&2
    exit 2
    ;;
esac
android_api=${ANDROID_NATIVE_API:-26}
android_tools="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
android_clang="$android_tools/$android_target$android_api-clang"
test -x "$android_clang"
android_output=${ANDROID_NATIVE_TARGET_DIR:-"$android_native/target"}
android_rustc=$(command -v rustc)
android_sysroot=$(rustc --print sysroot)
android_flags=()
if [[ ! -d "$android_sysroot/lib/rustlib/$android_target/lib" ]]; then
  android_source=${ANDROID_RUST_SOURCE:-"$android_sysroot/lib/rustlib/rustc-src/rust"}
  # A distribution's rust-src component is sometimes trimmed to the compiler's own
  # crates, and a Nix rustc is the common case: its `rustc-src` has no `library/`.
  # The `rust-src` package beside it does, so look there before giving up. Nix is
  # not required — the variable is still the way to point at any rust-src.
  if [[ ! -f "$android_source/library/Cargo.toml" ]]; then
    android_version=$(rustc --version | awk '{print $2}')
    for android_candidate in \
      "$android_sysroot"/../../*rust-src-"$android_version"-x86_64-unknown-linux-gnu/lib/rustlib/src/rust \
      /nix/store/*-rust-src-"$android_version"-x86_64-unknown-linux-gnu/lib/rustlib/src/rust \
      "$HOME"/.rustup/toolchains/*/lib/rustlib/rustc-src/rust; do
      if [[ -f "$android_candidate/library/Cargo.toml" ]]; then
        android_source="$android_candidate"
        break
      fi
    done
  fi
  if [[ ! -f "$android_source/library/Cargo.toml" ]]; then
    echo 'Install matching Android rust-std, or set ANDROID_RUST_SOURCE to matching rust-src (directory containing library/Cargo.toml).' >&2
    exit 2
  fi
  android_build_sysroot="$android_native/target/build-sysroot"
  mkdir -p "$android_build_sysroot/lib/rustlib/src"
  ln -sfn "$android_source" "$android_build_sysroot/lib/rustlib/src/rust"
  for android_std in "$android_sysroot"/lib/rustlib/*; do
    [[ -d "$android_std/lib" ]] || continue
    ln -sfn "$android_std" "$android_build_sysroot/lib/rustlib/$(basename "$android_std")"
  done
  export ANDROID_REAL_RUSTC="$android_rustc"
  export ANDROID_BUILD_SYSROOT="$android_build_sysroot"
  export RUSTC="$android_native/rustc-sysroot.sh"
  export RUSTC_BOOTSTRAP=1
  android_flags=(-Z build-std=std,panic_abort)
fi
android_upper=${android_target^^}
android_upper=${android_upper//-/_}
export "CARGO_TARGET_${android_upper}_LINKER=$android_clang"
export "CC_${android_target//-/_}=$android_clang"
export "AR_${android_target//-/_}=$android_tools/llvm-ar"
export "CARGO_TARGET_${android_upper}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384"
if [[ ! -f "$android_native/Cargo.lock" ]]; then
  cargo generate-lockfile --manifest-path "$android_native/Cargo.toml"
fi
cargo build --locked --manifest-path "$android_native/Cargo.toml" \
  --target-dir "$android_output" --target "$android_target" -j2 "${android_flags[@]}" "${android_profile_flags[@]}"
sha256sum "$android_output/$android_target/$android_profile/libmisa_android.so"
