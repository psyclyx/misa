{
  pkgs,
  apk,
  emulator,
}:
pkgs.writeShellApplication {
  name = "misa-android-check";
  runtimeInputs = [
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.android-tools
    pkgs.jdk21
  ];
  text = ''
    export ANDROID_HOME=${emulator.androidsdk}/libexec/android-sdk
    export MISA_TEST_APK=${apk}/share/misa/misa-debug-androidTest.apk
    export MISA_APK=${apk}/share/misa/misa-debug.apk
    ${builtins.readFile ../android/verify-install.sh}
  '';
}
