{ pkgs }:
{
  mkMisa = pkgs.callPackage ./mk-misa.nix {
    misa = pkgs.callPackage ./packages/misa.nix { };
  };
  standardExtensions = import ./standard-extensions.nix;
}
