{ callPackage, makeWrapper }:
let
  skia = callPackage ../skia.nix { };
in
(callPackage ../rust-package.nix { }) {
  pname = "misa-skia";
  nativeBuildInputs = skia.nativeBuildInputs ++ [ makeWrapper ];
  inherit (skia) buildInputs env preCheck;
  postFixup = ''
    wrapProgram "$out/bin/misa-skia" --set-default FONTCONFIG_FILE ${skia.env.FONTCONFIG_FILE} \
      --prefix LD_LIBRARY_PATH : ${skia.env.LD_LIBRARY_PATH}
  '';
}
