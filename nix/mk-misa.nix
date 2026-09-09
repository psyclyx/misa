{
  lib,
  misa,
  makeWrapper,
  symlinkJoin,
}:
{
  package ? misa,
  configuration ? null,
}:
let
  configFile =
    if configuration == null then
      null
    else if builtins.isPath configuration || (builtins.isString configuration && builtins.getContext configuration != { }) then
      "${configuration}"
    else
      throw "mkMisa: configuration must be a Nix path or a store path with dependency context";
in
symlinkJoin {
  name = "misa-with-extensions";
  paths = [ package ];
  nativeBuildInputs = [ makeWrapper ];
  postBuild = lib.optionalString (configFile != null) ''
    wrapProgram "$out/bin/misa" --set MISA_CONFIG ${lib.escapeShellArg (toString configFile)}
  '';
  passthru = {
    inherit configFile configuration;
    unwrapped = package;
  };
}
