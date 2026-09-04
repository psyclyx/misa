{ installPackage }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.misa;
  mkMisa = pkgs.callPackage ../mk-misa.nix { misa = cfg.package; };
  configured = mkMisa {
    inherit (cfg) extensions config;
    package = cfg.package;
  };
in
{
  options.programs.misa = {
    enable = lib.mkEnableOption "misa LuaJIT extension harness";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../packages/misa.nix { };
      defaultText = lib.literalExpression "pkgs.callPackage ./path/to/misa/nix/packages/misa.nix { }";
      description = "Unwrapped misa package to configure; no overlay is required.";
    };
    extensions = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [ ];
      description = "Ordered Lua extension script paths.";
    };
    config = lib.mkOption {
      type = lib.types.json;
      default = { };
      description = "Free-form JSON-serializable configuration exposed to Lua.";
    };
  };

  config = lib.mkIf cfg.enable (installPackage configured);
}
