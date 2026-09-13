# The legacy root shell reuses the rewrite's artifact-derived toolchain and tools.
# next/nix/shell.nix owns the composition, so there is no second dependency list.
{ pkgs }:
let
  shell = (import ../next { inherit pkgs; }).shell;
in
{
  packages = pkgs.lib.unique (shell.nativeBuildInputs ++ shell.buildInputs);
  env = builtins.intersectAttrs {
    SKIA_BINARIES_URL = null;
    FONTCONFIG_FILE = null;
    FONTCONFIG_PATH = null;
  } shell;
}
