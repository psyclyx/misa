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
in
mkShell {
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
  ];
  MISA_TREE_SITTER_DIR = treeSitterGrammars;
}
