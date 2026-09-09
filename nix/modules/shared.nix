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
    inherit (cfg) configuration;
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
    configuration = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Fennel or Lua file returning application data with config and definitions maps.";
    };
  };

  config = lib.mkIf cfg.enable (installPackage configured);
}
