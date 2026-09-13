# The rewrite's shell: bare `nix-shell` in this directory, or `nix-shell next -A shell` from the
# repository root (`default.nix` here is the same shell under the name a `.envrc` reads).
#
# The body is in `nix/shell.nix`, as a function of the toolchain it is given, because that is
# the shape a composition wants: this file decides *which nixpkgs* the toolchain comes from —
# the same pin as the rest of the repository, so a rustc here is not whatever a profile happens
# to have — and the body decides what is in the shell.
let
  pins = import ../npins;
  pkgs = import pins.nixpkgs { };
in
pkgs.callPackage ./nix/shell.nix { }
