{
  pkgs,
  mkShell,
  zig_0_16,
  luajit,
  pkg-config,
  treefmt,
  nixfmt,
  prettier,
}:
let
  treeSitterGrammars = pkgs.tree-sitter.withPlugins (_: pkgs.tree-sitter-grammars.allGrammars);
  # The rewrite's toolchain, from the same pin as everything else here — so that `cargo` in
  # this repository is this repository's rustc, wherever somebody is standing, and so that the
  # wasm tools the plugin host needs are on the path without a second shell.
  rust = import ./rust-toolchain.nix { inherit pkgs; };
in
mkShell (
  {
    packages = [
      zig_0_16
      luajit
      pkgs.fnlfmt
      pkgs.tree-sitter
      pkgs.libpng
      pkgs.libjpeg
      pkgs.sqlite
      pkg-config
      treefmt
      nixfmt
      prettier
    ]
    ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
      pkgs.wl-clipboard
      pkgs.xclip
      pkgs.xdg-utils
    ]
    ++ rust.packages;
    MISA_TREE_SITTER_DIR = treeSitterGrammars;
  }
  // rust.env
)
