{ lib, rustPlatform }:
{
  pname,
  crate ? pname,
  nativeBuildInputs ? [ ],
  buildInputs ? [ ],
  env ? { },
  ...
}@args:
rustPlatform.buildRustPackage (
  {
    inherit
      pname
      nativeBuildInputs
      buildInputs
      env
      ;
    version = "0.1.0";
    src = lib.cleanSourceWith {
      src = lib.cleanSource ../.;
      filter =
        path: _type:
        let
          relative = lib.removePrefix "${toString ../.}/" path;
        in
        !(builtins.elem (baseNameOf path) [
          "target"
          ".gradle"
          "build"
          ".direnv"
        ])
        && (
          path == toString ../.
          || builtins.elem relative [
            "Cargo.toml"
            "Cargo.lock"
            "crates"
            "wit"
          ]
          || lib.hasPrefix "crates/" relative
          || lib.hasPrefix "wit/" relative
        );
    };
    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = [
      "-p"
      crate
    ];
    cargoTestFlags = [
      "-p"
      crate
    ];
    strictDeps = true;
    meta = {
      description = "Misa session client and daemon";
      license = with lib.licenses; [
        mit
        asl20
      ];
      platforms = lib.platforms.unix;
      mainProgram = pname;
    };
  }
  // removeAttrs args [ "crate" ]
)
