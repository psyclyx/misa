# The same shell as `./shell.nix`, exposed under the name the root's `.envrc` reads
# (`use nix -A shell`), so that entering `next/` with direnv loads the rewrite's toolchain
# rather than only the repository's.
{
  shell = import ./shell.nix;
}
