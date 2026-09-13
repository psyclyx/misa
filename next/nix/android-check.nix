{
  pkgs,
  apk,
  emulator,
}:
pkgs.writeShellApplication {
  name = "misa-android-check";
  runtimeInputs = [
    pkgs.coreutils
    pkgs.android-tools
    pkgs.jdk21
  ];
  text = ''
    export ANDROID_HOME=${emulator.androidsdk}/libexec/android-sdk
    export MISA_APK=${apk}/share/misa/misa-debug.apk
    ${builtins.readFile ../android/verify-install.sh}
  '';
}
