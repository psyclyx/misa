{
  pkgs,
  lib,
  gradle2nixSource ? (import ../../../npins).gradle2nix,
}:
let
  android = import ../android.nix { inherit pkgs; };
  builders = pkgs.callPackage (gradle2nixSource + "/nix") { gradle = android.gradle; };
  sdkRoot = "${android.sdk.androidsdk}/libexec/android-sdk";
  apk = builders.buildGradlePackage {
    pname = "misa-android";
    version = "0.1.0";
    src = lib.cleanSourceWith {
      src = lib.cleanSource ../../android;
      filter =
        path: _:
        let
          relative = lib.removePrefix "${toString ../../android}/" path;
        in
        !(builtins.elem (baseNameOf path) [
          "build"
          ".gradle"
          "jniLibs"
          "local.properties"
        ])
        && (
          path == toString ../../android
          || builtins.elem relative [
            "build.gradle.kts"
            "settings.gradle.kts"
            "gradle.properties"
            "app"
          ]
          || lib.hasPrefix "app/" relative
        );

    };
    lockFile = ../../android/gradle.lock;
    gradle = android.gradle;
    buildJdk = pkgs.jdk21;
    nativeBuildInputs = [
      android.sdk.androidsdk
      pkgs.jdk21
      pkgs.unzip
    ];
    ANDROID_HOME = sdkRoot;
    ANDROID_SDK_ROOT = sdkRoot;
    postPatch = lib.concatStringsSep "\n" (
      lib.mapAttrsToList (abi: native: ''
        mkdir -p app/src/main/jniLibs/${abi}
        cp ${native}/lib/${abi}/libmisa_android.so app/src/main/jniLibs/${abi}/
      '') android.native
    );
    preBuild = ''
      export ANDROID_USER_HOME="$TMPDIR/android"
      mkdir -p "$ANDROID_USER_HOME"
    '';
    gradleBuildFlags = [
      "--offline"
      "--no-daemon"
      "-Pandroid.aapt2FromMavenOverride=${sdkRoot}/build-tools/37.0.0/aapt2"
      ":app:assembleDebug"
      ":app:testDebugUnitTest"
      ":app:assembleDebugAndroidTest"
    ];
    installPhase = ''
      runHook preInstall
      install -Dm644 app/build/outputs/apk/debug/app-debug.apk $out/share/misa/misa-debug.apk
      install -Dm644 app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk $out/share/misa/misa-debug-androidTest.apk
      ${sdkRoot}/build-tools/37.0.0/apksigner verify $out/share/misa/misa-debug.apk
      ${sdkRoot}/build-tools/37.0.0/apksigner verify $out/share/misa/misa-debug-androidTest.apk
      runHook postInstall
    '';
    passthru = {
      inherit (android) sdk emulator native;
      installCheck = import ../android-check.nix {
        inherit pkgs;
        inherit apk;
        inherit (android) emulator;
      };
    };
    meta = {
      description = "Misa Android client, debug-signed APK with all supported native ABIs";
      license = with lib.licenses; [
        mit
        asl20
      ];
      platforms = [ "x86_64-linux" ];
    };
  };
in
apk
