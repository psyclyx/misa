{
  package,
  lib,
  rustPlatform,
  lld,
  wasm-tools,
}:
rustPlatform.buildRustPackage {
  pname = "misa-${package}";
  version = "0.2.0";
  src = lib.cleanSourceWith {
    src = lib.cleanSource ../wit;
    filter = path: _type: baseNameOf path != "target";
  };
  cargoRoot = package;
  cargoLock.lockFile = ../wit + "/${package}/Cargo.lock";
  nativeBuildInputs = [
    lld
    wasm-tools
  ];
  buildPhase = ''
    runHook preBuild
    cargo build --manifest-path ${package}/Cargo.toml --target wasm32-unknown-unknown --target-dir target --release --offline --locked
    runHook postBuild
  '';
  doCheck = false;
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/lib/misa"
    wasm-tools component new target/wasm32-unknown-unknown/release/policy_${package}.wasm -o "$out/lib/misa/policy-${package}.wasm"
    wasm-tools validate "$out/lib/misa/policy-${package}.wasm"
    runHook postInstall
  '';
}
