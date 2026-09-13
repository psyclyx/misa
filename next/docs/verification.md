# Final integration verification

The implementation plan is complete and merged into local `main`. These results cover the final
application sources merged in `bb9e0d6`, X11 test isolation in `1fd83aa`, and the sandboxed Wayland
check configuration in `58a9bb9`. Subsequent documentation edits do not change artifact inputs.

- The ordinary workspace suite passes **580 tests**. One display-dependent clipboard test is
  ignored there and exercised separately under both display backends.
- `nix-build next -A packages.checks` passes **593 tests** with guest fixtures enabled, followed by
  Chromium DOM checks, native window interaction under Xvfb, and separate X11 and native Wayland
  text/PNG clipboard round trips. The Wayland check uses Sway with XWayland disabled and `DISPLAY`
  unset. Its output is `/nix/store/qgshjmibkhdq9sil404zbj5dkfvnwr1v-misa-checks-0.1.0`.
- The Linux terminal, daemon, browser, pixel and wasm guest product derivations build successfully.
  The guest fixture suite passes its 16 unit and 13 component tests. The daemon NixOS module's
  command quoting and state directory configuration are evaluated successfully.
- Android native libraries build for x86_64, arm64-v8a and armeabi-v7a, with ELF architecture,
  16 KiB alignment and API 26 NDK metadata checked. APK signatures and ZIP alignment pass.
  Seven native Rust tests pass. The exact packaged APK and daemon pass five Android 35 emulator
  scenarios: two incremental-state checks plus reconnect, blob transfer and directed file save.
  Emulator execution covers x86_64; the ARM outputs are build- and artifact-validated.
- Normal dependency graphs for terminal, browser, pixels and Android contain neither `misa-session`
  nor `misa-kernel`.

Native Wayland clipboard access requires compositor data-control support; XWayland remains a
fallback. Terminal selection copy uses OSC 52 and therefore also depends on terminal support.

Incremental work measurements and their limits are recorded in
[`incremental-evidence.md`](incremental-evidence.md),
[`pixel-retained-evidence.md`](pixel-retained-evidence.md) and
[`terminal-retained-evidence.md`](terminal-retained-evidence.md). They report measured work counts,
not elapsed-time speedups inferred from concurrent builds. Product subagents, a multi-session
registry and conversation branching remain the explicit exclusions in the plan.
