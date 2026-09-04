{
  lib,
  misa,
  makeWrapper,
  symlinkJoin,
  writeText,
}:
{
  package ? misa,
  extensions ? [ ],
  config ? { },
}:
let
  standardExtensions = import ./standard-extensions.nix;
  standardIds = builtins.attrValues standardExtensions;
  serializeExtension =
    extension:
    if builtins.isPath extension then
      # Interpolation copies path values to the store and retains string
      # context, making the generated configuration refer to its closure.
      "${extension}"
    else if builtins.isString extension && builtins.elem extension standardIds then
      extension
    else if builtins.isString extension then
      throw "mkMisa: unknown standard extension ID '${extension}'; expected one of ${lib.concatStringsSep ", " standardIds}, or a Nix path value"
    else
      throw "mkMisa: extensions must contain standard extension ID strings or Nix path values";
  serializedExtensions = map serializeExtension extensions;
  configData = {
    extensions = serializedExtensions;
    inherit config;
  };
  configFile = writeText "misa-config.json" (builtins.toJSON configData);
in
symlinkJoin {
  name = "misa-with-extensions";
  paths = [ package ];
  nativeBuildInputs = [ makeWrapper ];
  postBuild = ''
    wrapProgram "$out/bin/misa" --set MISA_CONFIG ${lib.escapeShellArg (toString configFile)}
  '';
  passthru = {
    inherit configData configFile extensions;
    unwrapped = package;
  };
}
