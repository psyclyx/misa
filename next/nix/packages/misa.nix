{
  callPackage,
  makeWrapper,
  misa-daemon,
}:
(callPackage ../rust-package.nix { }) {
  pname = "misa";
  crate = "misa-tui";
  nativeBuildInputs = [ makeWrapper ];
  postFixup = ''
    for bin in "$out/bin/misa" "$out/bin/misa-tui"; do
      wrapProgram "$bin" --set-default MISA_DAEMON_BIN ${misa-daemon}/bin/misa-daemon
    done
  '';
}
