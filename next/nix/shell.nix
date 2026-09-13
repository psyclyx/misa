# Build inputs come from the artifacts themselves. The remaining list is tooling.
{
  callPackage,
  mkShell,
  misa-daemon,
  misa,
  misa-web,
  misa-skia,
  misa-guest,
  misa-android,
  checks,
  rustfmt,
  clippy,
  rust-analyzer,
  cargo-nextest,
  treefmt,
  nixfmt,
  prettier,
  wasm-tools,
  wasmtime,
  wasm-component-ld,
  wabt,
}:
let
  skia = callPackage ./skia.nix { };
in
mkShell (
  {
    inputsFrom = [
      misa-daemon
      misa
      misa-web
      misa-skia
      misa-guest
      checks
      misa-android
    ]
    ++ builtins.attrValues misa-android.native;
    packages = [
      rustfmt
      clippy
      rust-analyzer
      cargo-nextest
      treefmt
      nixfmt
      prettier
      wasm-tools
      wasmtime
      wasm-component-ld
      wabt
      skia.dejavu_fonts
    ];
  }
  // skia.env
  // {
    inherit (misa-android) ANDROID_HOME ANDROID_SDK_ROOT;
  }
)
