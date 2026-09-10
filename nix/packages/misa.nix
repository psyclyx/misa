{
  pkgs,
  lib,
  stdenv,
  zig_0_16,
  pkg-config,
  makeWrapper,
  luajit,
}:
let
  treeSitterGrammars = pkgs.tree-sitter.withPlugins (_: pkgs.tree-sitter-grammars.allGrammars);
in
stdenv.mkDerivation {
  pname = "misa";
  version = "0.1.0";
  src = lib.cleanSourceWith {
    src = lib.cleanSource ../..;
    filter =
      path: _type:
      !(builtins.elem (baseNameOf path) [
        ".zig-cache"
        "zig-out"
        ".direnv"
      ]);
  };

  nativeBuildInputs = [
    zig_0_16
    pkg-config
    makeWrapper
    pkgs.buildPackages.luajit
  ];
  buildInputs = [
    luajit
    pkgs.tree-sitter
    pkgs.libpng
    pkgs.libjpeg
    pkgs.sqlite
  ];

  buildPhase = ''
    runHook preBuild
    export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-global-cache"
    export ZIG_LOCAL_CACHE_DIR="$TMPDIR/zig-local-cache"
    zig build -Doptimize=ReleaseSafe -Dtree-sitter-dir=${treeSitterGrammars} --prefix "$out"
    runHook postBuild
  '';
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    zig build test -Doptimize=ReleaseSafe -Dtree-sitter-dir=${treeSitterGrammars}
    runHook postCheck
  '';
  dontInstall = true;
  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    wrapProgram "$out/bin/misa" --prefix PATH : ${
      lib.makeBinPath [
        pkgs.wl-clipboard
        pkgs.xclip
        pkgs.xdg-utils
      ]
    }
  '';

  passthru.treeSitterGrammars = treeSitterGrammars;

  meta = {
    description = "Policy-free LuaJIT extension harness";
    license = lib.licenses.mit;
    mainProgram = "misa";
    platforms = lib.platforms.unix;
  };
}
