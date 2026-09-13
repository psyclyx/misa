{
  mkShell,
  rustc,
  cargo,
  clippy,
  rustfmt,
  rust-analyzer,
  pkg-config,
  # Skia's raster path links these. The scene needs neither, which is why `misa-skia`
  # has a `paint` feature: `cargo test --workspace` must not need a graphics stack.
  freetype,
  fontconfig,
  # A frontend with no typeface draws nothing, so the test that renders a PNG needs
  # one available.
  dejavu_fonts,
  # For the wasm plugin host when it lands.
  wasm-tools,
  cargo-nextest,
  treefmt,
  nixfmt,
}:
mkShell {
  packages = [
    rustc
    cargo
    clippy
    rustfmt
    rust-analyzer
    pkg-config
    freetype
    fontconfig
    dejavu_fonts
    wasm-tools
    cargo-nextest
    treefmt
    nixfmt
  ];

  # So `skia-bindings` finds the two libraries it links against without a
  # hand-written `-L`.
  PKG_CONFIG_PATH = "${freetype.dev}/lib/pkgconfig:${fontconfig.dev}/lib/pkgconfig";
  FONTCONFIG_FILE = "${fontconfig.out}/etc/fonts/fonts.conf";
}
