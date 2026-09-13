{
  pkgs,
  nixpkgs ? (import ../../npins).nixpkgs,
}:
let
  androidPkgs = import nixpkgs {
    inherit (pkgs.stdenv.hostPlatform) system;
    config = {
      android_sdk.accept_license = true;
      allowUnfree = true;
    };
  };
  sdk = androidPkgs.androidenv.composeAndroidPackages {
    buildToolsVersions = [ "37.0.0" ];
    cmdLineToolsVersion = "22.0";
    platformVersions = [
      "33"
      "34"
      "35"
      "36"
      "37"
    ];
    includeNDK = true;
    ndkVersions = [ "29.0.14206865" ];

  };
  emulator = androidPkgs.androidenv.composeAndroidPackages {
    platformVersions = [ "35" ];
    buildToolsVersions = [ "37.0.0" ];
    cmdLineToolsVersion = "22.0";
    includeEmulator = true;
    includeSystemImages = true;
    systemImageTypes = [ "google_apis" ];
    abiVersions = [ "x86_64" ];
  };
  gradle = pkgs.gradle-packages.mkGradle {
    version = "9.3.1";
    hash = "sha256-smbV/2uQ6tptw7IMsJDjcxMC5VOifF0+TfHw12vq/wY=";
    defaultJava = pkgs.jdk21;
  };
  targets = {
    "arm64-v8a" = {
      config = "aarch64-unknown-linux-android";
      rust.rustcTarget = "aarch64-linux-android";
    };
    "armeabi-v7a" = {
      config = "armv7a-unknown-linux-androideabi";
      rust.rustcTarget = "armv7-linux-androideabi";
    };
    "x86_64" = {
      config = "x86_64-unknown-linux-android";
      rust.rustcTarget = "x86_64-linux-android";
    };
  };
  native = pkgs.lib.mapAttrs (
    abi: target:
    let
      cross = import nixpkgs {
        localSystem = pkgs.stdenv.hostPlatform.system;
        crossSystem = target // {
          androidSdkVersion = "26";
          androidNdkVersion = "29";
          useAndroidPrebuilt = true;
        };
        config = {
          android_sdk.accept_license = true;
          allowUnfree = true;
        };
      };
    in
    cross.rustPlatform.buildRustPackage {
      pname = "misa-android-${abi}";
      version = "0.1.0";
      src = pkgs.lib.cleanSourceWith {
        src = pkgs.lib.cleanSource ../.;
        filter =
          path: _:
          let
            relative = pkgs.lib.removePrefix "${toString ../.}/" path;
          in
          !(builtins.elem (baseNameOf path) [
            "target"
            "build"
            ".gradle"
            "jniLibs"
            "local.properties"
          ])
          && (
            path == toString ../.
            || builtins.elem relative [
              "Cargo.toml"
              "Cargo.lock"
              "crates"
              "android"
              "android/native"
            ]
            || pkgs.lib.hasPrefix "crates/" relative
            || pkgs.lib.hasPrefix "android/native/" relative
          );
      };
      cargoRoot = "android/native";
      buildAndTestSubdir = "android/native";
      cargoLock.lockFile = ../android/native/Cargo.lock;
      doCheck = false; # Android executables run in the APK verification, not on the build host.
      CARGO_PROFILE_RELEASE_LTO = "thin";
      CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1";
      RUSTFLAGS = "-C link-arg=-Wl,-z,max-page-size=16384";
      installPhase = ''
        runHook preInstall
        install -Dm755 target/${target.rust.rustcTarget}/release/libmisa_android.so $out/lib/${abi}/libmisa_android.so
        runHook postInstall
      '';
      passthru = {
        inherit abi;
        rustVersion = cross.rustc.version;
      };
    }
  ) targets;
in
{
  inherit
    sdk
    emulator
    gradle
    native
    ;
}
