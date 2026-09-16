# Scoped refactor verification

This records the implementation described by [architecture.md](architecture.md) and
[refactor-plan.md](refactor-plan.md). Runtime code is `bb386a4` plus the mobile web heading fix
`e370b7a`. Fixture packaging (`95e76b6`) and native acceptance-script corrections (`69d398f`)
change test/build definitions. Documentation edits do not affect artifact inputs. Earlier
verification in repository history describes older artifacts.

## Automated gates

- **752 workspace tests passed**, with one ignored display-dependent clipboard measurement.
  The suite covers coherent selection/recovery, incremental rendering, precommit admission
  rollback, accepted completion under saturation, disconnect-safe writes and awaited shutdown.
  Log: `/tmp/misa-refactor-certified-workspace.log`.
- All 18 actual Wasm integration tests pass with both pinned components after fixing the pet
  fixture's missing sandbox dependencies (`/tmp/misa-pinned-fixtures-tests.log`). No guest tests
  were disabled. The final pinned package run passes **770 Rust tests**, including those 18
  component tests, plus browser DOM and native-window checks. The separate text/PNG clipboard
  test passes under both X11 and native Wayland with `DISPLAY` unset. The complete package
  finished with exit 0 (`/tmp/misa-refactor-isolated-checks.log`). Earlier interrupted attempts
  were rerun in an isolated process session; no failures were skipped.
- Android native libraries build for x86_64, arm64-v8a and armeabi-v7a; host Rust and Kotlin checks
  pass. **All 10 expanded emulator scenarios pass against the final packaged daemon** through the
  production check wrapper (`/tmp/misa-android-final-server-check.log`). They cover provider
  enumeration, private forms, local preferences, multiple daemons, exact request routing,
  renderer rebaseline, attachment transfer and daemon restart with a refreshed endpoint address.
- Normal production dependency graphs for terminal, browser, native pixels and Android contain
  no session, daemon, kernel or plugin runtime. The optional kit has no renderer dependency.

## Packaged interaction gates

| Gate              | Demonstrated behavior                                                                                                      | Evidence                                                                                   |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| Terminal PTY      | Print EOF leaves daemon running; empty/nonempty Ctrl-D; picker paste in composer; daemon Ctrl-C                            | `/tmp/misa-certified-pty.log`, `/tmp/misa-pty-lifecycle-2poo0_3i`                          |
| Native appearance | Installed command discovery and persisted light-theme pixels                                                               | `/tmp/misa-certified-native-appearance.log`, `/tmp/misa-native-workspace-test-c6bijI`      |
| Native forms      | Pinned pet component, typed response, operation completion and cancellation                                                | `/tmp/misa-certified-native-request.log`, `/tmp/misa-native-workspace-test-WeJQsa`         |
| Native workspace  | Two daemons, independent drafts, reconnect, credential clearing, tool approval and presentation composition                | `/tmp/misa-certified-native-workspace-final.log`, `/tmp/misa-native-workspace-test-GONtfR` |
| Native lifecycle  | Create, stream, close, recover retained draft, browse archive and resume authoritative session                             | `/tmp/misa-certified-native-lifecycle-final.log`, `/tmp/misa-native-workspace-test-X7yKcE` |
| Browser lifecycle | Local discovery, generic daemon forms, stream, independent reload/draft/theme, close/archive/resume, desktop/mobile layout | `/tmp/misa-web-final-mobile-lifecycle.log`, `/tmp/misa-web-lifecycle-29Tnz2`               |
| Android emulator  | Exact packaged APK and daemon, ten expanded scenarios and screenshot review                                                | `/tmp/misa-android-final-server-check.log`, `/tmp/misa-android-final-server-ui`            |

All interaction gates above exited successfully. Screenshots were inspected, including the final
mobile web heading, persisted native theme, resolved pet form, tool approval and resumed history.
Browser DOM checks also pass after the final CSS adjustment (`/tmp/misa-web-mobile-dom.log`).

## Exact artifacts

| Artifact              | Nix store output                                                 |
| --------------------- | ---------------------------------------------------------------- |
| Complete checks       | `/nix/store/4mv7nzzpa63a7yp8rmz4kwpkd17jv6c9-misa-checks-0.1.0`  |
| Daemon                | `/nix/store/93p242r9qjpif9r8bkicynlidc437dz8-misa-daemon-0.1.0`  |
| Terminal              | `/nix/store/sh3d1wg6wxrkb0ps1slxrap8hqah6gfy-misa-0.1.0`         |
| Browser client        | `/nix/store/rz88p91hqwn9jb7fq1dsyl5ggjww8bnl-misa-web-0.1.0`     |
| Native pixels         | `/nix/store/c3vfkpj8gl2ychv5kwypp6s7akif6rww-misa-skia-0.1.0`    |
| Policy guest          | `/nix/store/5q3yka2vp1p5smfrp3h4f19i5bvizfrm-misa-guest-0.2.0`   |
| Pet guest             | `/nix/store/4v7bzv1wmccc7n2g033iaqd0wxpnyp59-misa-pet-0.2.0`     |
| Android APK           | `/nix/store/rs9kfbljdxfrql5a1bdni8vj9az5mglg-misa-android-0.1.0` |
| Android check wrapper | `/nix/store/wzgckrpsvfhqfgj8zbq7nmla9zlnafx0-misa-android-check` |

Product derivations include their package tests. Only the browser runtime changed after the
initial runtime freeze, and its final artifact was rebuilt and rechecked. The Android wrapper
references the exact frozen APK and final daemon; it does not substitute a development binary.

## Platform and measurement limits

Android emulator execution covers x86_64; ARM outputs are build- and artifact-validated. Native
Wayland clipboard access requires compositor data-control support; XWayland remains a fallback.
Terminal selection copy uses OSC 52 and depends on terminal support. Native window tests use Xvfb;
they do not establish identical behavior on every window manager.

Incremental work evidence is in [incremental-evidence.md](incremental-evidence.md),
[pixel-retained-evidence.md](pixel-retained-evidence.md) and
[terminal-retained-evidence.md](terminal-retained-evidence.md). These are work-count measurements,
not elapsed-time speedups inferred from concurrent builds. Refactor regression tests preserve
incremental tree, append and retained-renderer behavior. Cross-owner observations remain
independently versioned; no gate claims a distributed atomic snapshot.
