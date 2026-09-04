{
  mkShell,
  zig_0_16,
  luajit,
  pkg-config,
  treefmt,
  nixfmt,
  prettier,
}:
mkShell {
  packages = [
    zig_0_16
    luajit
    pkg-config
    treefmt
    nixfmt
    prettier
  ];
}
