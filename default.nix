let
  npins = import ./npins;

  nextOverlay = (import ./next { }).overlay;
  overlay =
    final: prev:
    nextOverlay final prev
    // {
      misa-legacy = final.callPackage ./nix/packages/misa.nix { };
    };
in
{
  nixpkgs ? npins.nixpkgs,
  pkgs ? import nixpkgs { },
}:
let
  finalPkgs = pkgs.extend overlay;
  packages = overlay finalPkgs pkgs;
  misaLib = import ./nix/lib.nix { pkgs = finalPkgs; };
  nixosModule = import ./nix/modules/nixos.nix;
  darwinModule = import ./nix/modules/darwin.nix;
  homeManagerModule = import ./nix/modules/home-manager.nix;
in
{
  inherit packages overlay;
  pkgs = finalPkgs;
  default = packages.misa;
  shell = finalPkgs.callPackage ./nix/shell.nix { };
  lib = misaLib;
  standardExtensions = misaLib.standardExtensions;

  nixosModules = {
    misa = nixosModule;
    misa-daemon = import ./next/nix/daemon-module.nix;
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
