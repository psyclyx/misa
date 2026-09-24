let
  pins = import ../npins;
  mkPackages =
    lib: pkgs: gradle2nixSource:
    lib.packagesFromDirectoryRecursive {
      callPackage = lib.callPackageWith (pkgs // { inherit gradle2nixSource; });
      directory = ./nix/packages;
    };
in
{
  sources ? pins,
  nixpkgs ? sources.nixpkgs,
  # External dep — the gradle2nix builders used for misa-android.
  gradle2nix ? sources.gradle2nix,
  pkgs ? import nixpkgs { },
  ...
}:
let
  # Surface gradle2nix by name so misa-android's `gradle2nixSource`
  # callPackage arg resolves it (the shoal pattern).
  overlay = final: prev: mkPackages prev.lib final gradle2nix;
  finalPkgs = pkgs.extend overlay;
in
rec {
  inherit overlay;
  packages = mkPackages pkgs.lib finalPkgs gradle2nix;
  default = packages.misa;
  shell = finalPkgs.callPackage ./nix/shell.nix { };
}
