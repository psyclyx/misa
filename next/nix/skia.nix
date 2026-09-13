{
  fetchurl,
  pkg-config,
  freetype,
  fontconfig,
  dejavu_fonts,
  makeFontsConf,
}:
let
  # The archive is the exact Skia build paired with skia-safe in Cargo.lock. A
  # fixed-output input keeps the crate's build script offline, including in the shell.
  binaries = fetchurl {
    url = "https://github.com/rust-skia/skia-binaries/releases/download/0.93.1/skia-binaries-319323662b1685a112f5-x86_64-unknown-linux-gnu-jpegd-jpege-pdf-textlayout.tar.gz";
    sha256 = "0dyzzwzm5x7z1yszqy6v541q16iha7p8qajhdqlgpwpm4sbk2z88";
  };
in
{
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [
    freetype
    fontconfig
  ];
  preCheck = ''
    export XDG_CACHE_HOME="$TMPDIR/font-cache"
    mkdir -p "$XDG_CACHE_HOME"
  '';
  env = {
    SKIA_BINARIES_URL = "file://${binaries}";
    FONTCONFIG_FILE = makeFontsConf { fontDirectories = [ dejavu_fonts ]; };
    FONTCONFIG_PATH = "${fontconfig.out}/etc/fonts";
  };
  inherit binaries dejavu_fonts;
}
