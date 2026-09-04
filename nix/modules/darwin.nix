import ./shared.nix {
  installPackage = package: { environment.systemPackages = [ package ]; };
}
