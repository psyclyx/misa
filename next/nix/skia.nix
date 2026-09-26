{
  fetchurl,
  stdenv,
  pkg-config,
  freetype,
  fontconfig,
  dejavu_fonts,
  makeFontsConf,
  lib,
  libX11,
  libXcursor,
  libXi,
  libXrandr,
  libxkbcommon,
  wayland,
  vulkan-loader,
  mesa,
}:
# Upstream's prebuilt Vulkan archive and the headless test ICD are for this host.
# Do not silently supply x86_64 binaries to a different target.
assert stdenv.hostPlatform.system == "x86_64-linux";
let
  # skia-safe =0.93.1 in Cargo.toml enables default PDF/ICU plus textlayout and
  # Vulkan. Nix fetches rust-skia's matching upstream artifact for its offline
  # sandbox; Cargo outside Nix can use rust-skia's normal binary-cache download.
  binaries = fetchurl {
    url = "https://github.com/rust-skia/skia-binaries/releases/download/0.93.1/skia-binaries-319323662b1685a112f5-x86_64-unknown-linux-gnu-jpegd-jpege-pdf-textlayout-vulkan.tar.gz";
    sha256 = "1n6dca3rzgjpcvjvr4czd0ivw5frlnfjrqmsqr087qvchflzynlc";
  };
  headlessLibraries = [
    freetype
    fontconfig
    vulkan-loader
  ];
  windowLibraries = [
    libX11
    libXcursor
    libXi
    libXrandr
    libxkbcommon
    wayland
  ];
  headlessEnv = {
    SKIA_BINARIES_URL = "file://${binaries}";
    FONTCONFIG_FILE = makeFontsConf { fontDirectories = [ dejavu_fonts ]; };
    FONTCONFIG_PATH = "${fontconfig.out}/etc/fonts";
    LD_LIBRARY_PATH = lib.makeLibraryPath [ vulkan-loader ];
  };
in
{
  nativeBuildInputs = [ pkg-config ];
  headlessBuildInputs = headlessLibraries;
  buildInputs = headlessLibraries ++ windowLibraries;
  preCheck = ''
    export XDG_CACHE_HOME="$TMPDIR/font-cache"
    mkdir -p "$XDG_CACHE_HOME"
    # Headless Vulkan tests use the same Ganesh path as the window, without
    # depending on a host GPU or a display server in the build sandbox.
    export VK_ICD_FILENAMES=${mesa}/share/vulkan/icd.d/lvp_icd.x86_64.json
  '';
  inherit headlessEnv;
  env = headlessEnv // {
    LD_LIBRARY_PATH = lib.makeLibraryPath (headlessLibraries ++ windowLibraries);
  };
  inherit binaries dejavu_fonts mesa;
}
