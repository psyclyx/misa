let
  pins = import ../npins;
  mkPackages =
    lib: pkgs:
    lib.packagesFromDirectoryRecursive {
      inherit (pkgs) callPackage;
      directory = ./nix/packages;
    };
  overlay = final: prev: mkPackages prev.lib final;
in
{
  nixpkgs ? pins.nixpkgs,
  pkgs ? import nixpkgs { },
}:
let
  finalPkgs = pkgs.extend overlay;
in
rec {
  inherit overlay;
  packages = mkPackages pkgs.lib finalPkgs;
  default = packages.misa;
  shell = finalPkgs.callPackage ./nix/shell.nix { };
}
