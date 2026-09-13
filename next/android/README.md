# misa for Android

A phone frontend for a misa daemon: the same session, the same commands, the
same transcript, reached over iroh with the same ticket or pairing code every
other client takes.

It is a frontend and not a second implementation. The Kotlin side owns the
surface — a `Compose` tree instead of cells, HTML, or pixels — and the protocol
is the shared Rust client (`misa-net`, `misa-proto`, `misa-client`), reached
through a small JNI seam in `native/`. One connection per session, one view
tree, one intent vocabulary.

```
        ┌─────────────── app (Kotlin, Compose) ───────────────┐
        │  connect · scan the code · transcript · composer    │
        └───────────────────────┬────────────────────────────┘
                                │ JNI, JSON both ways
        ┌───────────────────────┴────────────────────────────┐
        │  native/libmisa_android.so  (misa-net + iroh)       │
        └───────────────────────┬────────────────────────────┘
                                │  /misa/session/1
                            misa-daemon
```

## Build

From the repository root:

```sh
nix-build next -A packages.misa-android
```

The output is `result/share/misa/misa-debug.apk`, debug-signed and containing
`x86_64`, `arm64-v8a`, and `armeabi-v7a` native libraries. Each library is its own
cross derivation with Rust 1.97.1 and NDK 29.0.14206865. The APK uses Gradle 9.3.1,
build tools 37.0.0, and the pinned SDK platforms 33–37. Dependencies are fetched
from `gradle.lock`; Gradle assembles with networking disabled.

To build only a native library:

```sh
nix-build next -A packages.misa-android.native.x86_64
```

To verify installation and launch in a temporary x86_64 emulator (requires KVM):

```sh
nix-build next -A packages.misa-android.installCheck -o result-android-check
result-android-check/bin/misa-android-check
```

When Gradle dependencies change, regenerate `gradle.lock` with the pinned
`gradle2nix`, running `:app:assembleDebug` and `:app:testDebugUnitTest`. Gradle
consumes the native libraries supplied by Nix; it does not discover compilers
or build Rust as a hidden `preBuild` task.

## Run

The daemon shows a code; scan it, or paste the `misa-pair:` line:

```sh
misa-daemon --session demo            # prints a ticket and a QR
adb install -r result/share/misa/misa-debug.apk # then attach with the camera
```

An emulator on this machine can reach a daemon on this machine at its loopback
address, which is what `misa-daemon` prints when it binds without a relay.

## What it does today

- attach with a ticket or a pairing code, and pair once so the key is approved;
- the transcript, drawn from the same view tree the other frontends draw:
  messages, thinking, tool calls, code with captures, lists, tables, fields,
  meters, and typed facts;
- a composer that sends a turn or a `/command`;
- the command palette from the session's own declarations, with an argument
  form built from what each command declared;
- the session's own actions, including a panel's fields;
- notices, status, and cancel.

Not yet: fetching images over the blob connection, and keeping the last session
across a restart. The session emits semantic image nodes; the phone currently
renders their alternative text.
