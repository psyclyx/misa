{
  callPackage,
  misa-guest,
  wasm-tools,
  lld,
  chromium,
  xorg-server,
  xdotool,
  xclip,
}:
let
  skia = callPackage ../skia.nix { };
in
(callPackage ../rust-package.nix { }) {
  pname = "misa-checks";
  nativeBuildInputs = skia.nativeBuildInputs ++ [
    wasm-tools
    lld
    chromium
    xorg-server
    xdotool
    xclip
  ];
  inherit (skia) buildInputs preCheck;
  env = skia.env // {
    MISA_PLUGIN_FIXTURE = "${misa-guest}/lib/misa/policy-guest.wasm";
  };
  cargoBuildFlags = [ "--workspace" ];
  cargoTestFlags = [
    "--workspace"
    "--features"
    "misa-plugin/guest-fixture"
  ];
  postCheck = ''
    bash crates/misa-web/tests/browser.sh
    bash crates/misa-skia/tests/window.sh target/release/misa-skia
    bash crates/misa-tui/tests/clipboard.sh --release --offline
  '';
  installPhase = ''
    mkdir -p "$out"
    printf '%s\n' 'workspace and guest fixture tests passed' > "$out/passed"
  '';
}
