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
    pkgs.tree-sitter
    pkg-config
    treefmt
    nixfmt
    prettier
  ];
  MISA_TREE_SITTER_DIR = treeSitterGrammars;
}
