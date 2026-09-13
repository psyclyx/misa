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

The native library is built by the Android NDK through an explicit script, and
Gradle runs it as part of `preBuild`, so one command is the whole build:

```sh
export ANDROID_HOME=/path/to/android-sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/29.0.14206865
./gradlew :app:assembleDebug                 # x86_64, for an emulator
./gradlew :app:assembleDebug -PmisaAbis=arm64-v8a   # a real phone
```

### On NixOS

The `aapt2` AGP downloads is a generic Linux executable and will not start on a
distribution whose loader is not where it expects. The SDK beside it already has
one that runs, and AGP has an option to prefer it:

```sh
./gradlew :app:assembleDebug \
  -Pandroid.aapt2FromMavenOverride="$ANDROID_HOME/build-tools/37.0.0/aapt2"
```

### The Rust side alone

```sh
cd native
ANDROID_NDK_HOME=$ANDROID_NDK_HOME ./build.sh x86_64-linux-android
```

`build.sh` needs an Android `rust-std`. When the toolchain does not have one it
builds `core`/`std` from `rust-src` with `-Z build-std`, and finds a `rust-src`
beside a Nix `rustc` or under `~/.rustup`. `ANDROID_RUST_SOURCE` overrides the
search.

## Run

The daemon shows a code; scan it, or paste the `misa-pair:` line:

```sh
misa-daemon --session demo            # prints a ticket and a QR
./gradlew :app:installDebug           # then attach with the camera
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

Not yet: fetching images over the blob connection (`graphics` is off, so the
session sends words where a picture would be, exactly as it does for the
terminal), and keeping the last session across a restart. Both are additive and
neither needs a protocol change.
