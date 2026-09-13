# The shell `next/` is worked on in: the Rust toolchain the rewrite is built with, and the wasm
# tools the plugin host needs. Bare `nix-shell` in this directory, or `nix-shell next -A shell`
# from the repository root. `../../shell.nix` beside it is the file that decides which nixpkgs
# these come from.
#
# The toolchain itself is in `nix/rust-toolchain.nix` at the repository root, shared with the
# root shell — one answer to "which rustc", for both halves of this repository.
{
  pkgs,
  mkShell,
  treefmt,
  nixfmt,
  prettier,
}:
let
  rust = import ../../nix/rust-toolchain.nix { inherit pkgs; };
in
mkShell (
  {
    packages = rust.packages ++ [
      # The pre-commit hook is treefmt over this tree, and a hook that is not on the path is a
      # commit that fails for a reason that has nothing to do with the change.
      treefmt
      nixfmt
      prettier
    ];
  }
  // rust.env
)
