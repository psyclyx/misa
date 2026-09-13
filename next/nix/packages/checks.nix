{
  callPackage,
  stdenv,
  misa-guest,
  wasm-tools,
  lld,
  chromium,
  xorg-server,
  xdotool,
  xclip,
  sway,
  wl-clipboard,
  dbus,
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
    sway
    wl-clipboard
    dbus
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
    bash crates/misa-skia/tests/window.sh target/${stdenv.hostPlatform.rust.rustcTarget}/release/misa-skia
    bash crates/misa-tui/tests/clipboard.sh --release --offline --target ${stdenv.hostPlatform.rust.rustcTarget}
    ${dbus}/bin/dbus-run-session --config-file=${dbus}/share/dbus-1/session.conf -- \
      bash crates/misa-tui/tests/clipboard-wayland.sh --release --offline --target ${stdenv.hostPlatform.rust.rustcTarget}
  '';
  installPhase = ''
    mkdir -p "$out"
    printf '%s\n' 'workspace and guest fixture tests passed' > "$out/passed"
  '';
}
