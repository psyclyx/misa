{ callPackage }:
(callPackage ../rust-package.nix { }) { pname = "misa-daemon"; }
