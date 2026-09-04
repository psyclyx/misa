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
  configFile = writeText "misa-config.json" (
    builtins.toJSON {
      extensions = map toString extensions;
      inherit config;
    }
  );
in
symlinkJoin {
  name = "misa-with-extensions";
  paths = [ package ];
  nativeBuildInputs = [ makeWrapper ];
  postBuild = ''
    wrapProgram "$out/bin/misa" --set MISA_CONFIG ${lib.escapeShellArg (toString configFile)}
  '';
  passthru = {
    inherit configFile extensions;
    unwrapped = package;
  };
}
