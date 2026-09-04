import ./shared.nix {
  installPackage = package: { home.packages = [ package ]; };
}
