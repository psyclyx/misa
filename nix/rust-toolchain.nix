# The Rust toolchain, and the wasm pieces the plugin host needs.
#
# One file because two shells want it: `nix-shell` at the repository root — which is what
# `.envrc` loads, so a stray `cargo` anywhere in this repository is the toolchain this
# repository pins and not whatever is in somebody's profile — and `nix-shell next`, which is
# the rewrite on its own. A second copy would be a second answer to "which rustc".
#
# # Why the wasm target is `wasm32-unknown-unknown` and not `wasm32-wasip2`
#
# nixpkgs' `rustc` builds std for the host, for `wasm32-unknown-unknown`, and for
# `wasm32v1-none`; `wasm32-wasip2` would mean a toolchain from somewhere else. Nothing here
# needs it: a policy plugin imports no WASI. It exports handlers and answers queries, and
# `wit-bindgen` emits the canonical ABI *and* the `component-type` custom section for a plain
# `wasm32-unknown-unknown` module, which `wasm-tools component new` turns into a component.
# So the missing target costs nothing, and the two commands that make a component are in this
# shell: `cargo build --target wasm32-unknown-unknown` then `wasm-tools component new`.
#
# # What each wasm tool is for
#
# - `wasm-tools` — the one that matters: `component new` (module + wit → component),
#   `validate`, `print`, and `component wit` (what a component *says* it exports).
# - `wasmtime` — the reference runtime, as a command, for running a component by hand before
#   the host exists and for telling a host bug from a guest bug afterwards.
# - `wasm-component-ld` — the linker rustc uses when a target produces components directly.
#   Unused today, and the reason `wasm32-wasip2` would work if that target ever arrives.
# - `wabt` — `wasm2wat`: reading what the compiler emitted, which is how a canonical ABI
#   question gets answered rather than guessed.
#
# # Why lld is here
#
# nixpkgs builds rustc with `--disable-lld`, so `rust-lld` — the linker rustc reaches for
# when the target is wasm, because the target has no system linker — is not in the toolchain.
# Without it a wasm build stops at "linker `lld` not found", which looks like a target
# problem and is a packaging one. `lld` provides `wasm-ld` as well as `ld.lld`.
{ pkgs }:
{
  packages = [
    pkgs.rustc
    pkgs.cargo
    pkgs.clippy
    pkgs.rustfmt
    pkgs.rust-analyzer
    pkgs.pkg-config
    pkgs.lld
    pkgs.wasm-tools
    pkgs.wasmtime
    pkgs.wasm-component-ld
    pkgs.wabt
    pkgs.cargo-nextest
    # `misa-skia`'s raster path links these two, which is exactly why `paint` is a feature:
    # the workspace test run must not need a graphics stack, and the one test that draws a
    # picture needs one available.
    pkgs.freetype
    pkgs.fontconfig
    pkgs.dejavu_fonts
  ];

  env = {
    # So `skia-bindings` finds the two libraries it links against without a hand-written `-L`.
    PKG_CONFIG_PATH = "${pkgs.freetype.dev}/lib/pkgconfig:${pkgs.fontconfig.dev}/lib/pkgconfig";
    FONTCONFIG_FILE = "${pkgs.fontconfig.out}/etc/fonts/fonts.conf";
  };
}
