{
  lib,
  rustPlatform,
  lld,
  wasm-tools,
}:
rustPlatform.buildRustPackage {
  pname = "misa-guest";
  version = "0.2.0";
  src = lib.cleanSourceWith {
    src = lib.cleanSource ../../wit;
    filter = path: _type: baseNameOf path != "target";
  };
  cargoRoot = "guest";
  cargoLock.lockFile = ../../wit/guest/Cargo.lock;
  nativeBuildInputs = [
    lld
    wasm-tools
  ];
  buildPhase = ''
    runHook preBuild
    cargo build --manifest-path guest/Cargo.toml --target wasm32-unknown-unknown --target-dir target --release --offline --locked
    runHook postBuild
  '';
  doCheck = false;
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/lib/misa"
    wasm-tools component new target/wasm32-unknown-unknown/release/policy_guest.wasm -o "$out/lib/misa/policy-guest.wasm"
    wasm-tools validate "$out/lib/misa/policy-guest.wasm"
    runHook postInstall
  '';
}
