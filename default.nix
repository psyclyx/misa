let
  npins = import ./npins;

  mkPackages = pkgs: {
    misa = pkgs.callPackage ./nix/packages/misa.nix { };
  };

  overlay = final: _prev: mkPackages final;
in
{
  nixpkgs ? npins.nixpkgs,
  pkgs ? import nixpkgs { },
}:
let
  finalPkgs = pkgs.extend overlay;
  packages = mkPackages finalPkgs;
  misaLib = import ./nix/lib.nix { pkgs = finalPkgs; };
  nixosModule = import ./nix/modules/nixos.nix;
  darwinModule = import ./nix/modules/darwin.nix;
  homeManagerModule = import ./nix/modules/home-manager.nix;
in
{
  inherit packages overlay;
  default = packages.misa;
  shell = finalPkgs.callPackage ./nix/shell.nix { };
  lib = misaLib;
  standardExtensions = misaLib.standardExtensions;

  nixosModules = {
    misa = nixosModule;
    default = nixosModule;
  };
  darwinModules = {
    misa = darwinModule;
    default = darwinModule;
  };
  homeManagerModules = {
    misa = homeManagerModule;
    default = homeManagerModule;
  };
  modules = {
    nixos = nixosModule;
    darwin = darwinModule;
    home-manager = homeManagerModule;
  };
}
