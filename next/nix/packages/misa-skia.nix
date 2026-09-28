{
  callPackage,
  makeWrapper,
  misa-daemon,
}:
let
  skia = callPackage ../skia.nix { };
in
(callPackage ../rust-package.nix { }) {
  pname = "misa-skia";
  nativeBuildInputs = skia.nativeBuildInputs ++ [ makeWrapper ];
  inherit (skia) buildInputs env preCheck;
  postFixup = ''
    wrapProgram "$out/bin/misa-skia" --set-default FONTCONFIG_FILE ${skia.runtimeEnv.FONTCONFIG_FILE} \
      --set-default MISA_DAEMON_BIN ${misa-daemon}/bin/misa-daemon \
      --prefix LD_LIBRARY_PATH : ${skia.env.LD_LIBRARY_PATH}
  '';
}
