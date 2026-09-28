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
    # The dev shell runs and tests the app against the machine's own fonts;
    # nix builds keep skia.env's pinned DejaVu for reproducible tests.
    FONTCONFIG_FILE = skia.runtimeEnv.FONTCONFIG_FILE;
    inherit (misa-android) ANDROID_HOME ANDROID_SDK_ROOT;
  }
)
