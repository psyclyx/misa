{
  callPackage,
  misa-guest,
  wasm-tools,
  lld,
  chromium,
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
  '';
  installPhase = ''
    mkdir -p "$out"
    printf '%s\n' 'workspace and guest fixture tests passed' > "$out/passed"
  '';
}
