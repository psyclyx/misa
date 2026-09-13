{ callPackage }:
(callPackage ../rust-package.nix { }) {
  pname = "misa";
  crate = "misa-tui";
}
