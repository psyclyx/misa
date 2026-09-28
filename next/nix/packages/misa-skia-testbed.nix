{
  callPackage,
  makeWrapper,
}:
let
  skia = callPackage ../skia.nix { };
in
(callPackage ../rust-package.nix { }) {
  pname = "misa-skia-testbed";
  nativeBuildInputs = skia.nativeBuildInputs ++ [ makeWrapper ];
  buildInputs = skia.headlessBuildInputs;
  env = skia.headlessEnv;
  inherit (skia) preCheck;
  postFixup = ''
    wrapProgram "$out/bin/misa-skia-testbed" --set-default FONTCONFIG_FILE ${skia.runtimeEnv.FONTCONFIG_FILE} \
      --set-default VK_ICD_FILENAMES ${skia.mesa}/share/vulkan/icd.d/lvp_icd.x86_64.json \
      --prefix LD_LIBRARY_PATH : ${skia.headlessEnv.LD_LIBRARY_PATH}
  '';
}
