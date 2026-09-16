# misa for Android

Android uses `misa-client` for daemon relationships, scoped observations,
replicas, recovery, command outcomes and transfers. Kotlin owns Compose rendering,
local session selection, drafts, dialogs and platform file pickers. The JNI seam
adapts a bounded workspace command queue and coalesced wakeups; it is not another
wire protocol implementation.

One workspace can connect to several daemons and retain several session pages.
A session is identified by authenticated daemon identity and its full owner
scope, including incarnation. Reconnecting never silently substitutes another
session with the same name. Phone connections use explicit tickets or pairing
codes; desktop Unix socket discovery does not apply to Android.

## Build and verify

From the repository root:

```sh
nix-build next -A packages.misa-android
```

The output contains `share/misa/misa-debug.apk` and its instrumentation APK,
debug-signed together. The primary APK contains `x86_64`, `arm64-v8a`, and
`armeabi-v7a` native libraries, built as separate Nix cross derivations. The
pinned SDK, NDK, Rust and Gradle dependencies are declared in `next/nix/android.nix`.
Gradle assembles offline and does not build Rust as a hidden pre-build task.

```sh
# One ABI only
nix-build next -A packages.misa-android.native.x86_64
# Host-side JNI support tests
cargo test --manifest-path next/android/native/Cargo.toml
# Actual packaged JNI/iroh/Kotlin checks in a temporary emulator (requires KVM)
nix-build next -A packages.misa-android.installCheck -o result-android-check
result-android-check/bin/misa-android-check
```

The emulator check starts isolated scripted daemons, installs both APKs, and
checks incremental canonical/live rendering, persistent identity and reconnect,
offline canonical history, verified image transfer and directed attachment save,
owner summaries, create/close/archive/resume, two daemon identities with the same
session name, and private credential/tool approval workflows. It cleans up only
its own processes and temporary files. `MISA_CHECK_ARTIFACTS=/tmp/misa-android-ui`
also saves an actual application screenshot and UI hierarchy.

For the contributed typed-request fixture, build the component as described in
`next/wit/pet/README.md`, then set `MISA_PLUGIN` to its component path when running
the check. This adds a real installed command, invalid integer draft rejection,
and successful owner-owned request resolution through JNI.

When Gradle dependencies change, regenerate `gradle.lock` with pinned
`gradle2nix`, including `:app:assembleDebug`, `:app:testDebugUnitTest`, and
`:app:assembleDebugAndroidTest`.

## Run

Scan the daemon's pairing code or paste its ticket. An emulator reaches its host
at `10.0.2.2`; replace `127.0.0.1` in a direct ticket with that address.

The workspace chooser adds daemons, selects sessions, creates or resumes an
archived conversation, closes an owner session, and separately detaches local
pages. The daemon computes working, attention and usage summaries; Android does
not open every transcript to derive them.

The session command palette includes declared shortcuts and installed command
input forms. Optional presentations are selected before observation, with local
persisted hide/auto/variant preferences. Their trees have independent identity
spaces. Private input requests are opened locally, retain nonsecret drafts when
hidden, and clear secret text on dismissal or submission. Operations retain their
owner lifetime when a local dialog or page closes.

## Delivery and storage

The shared replica advances independently of rendering. JNI queues one deferred
capture per document composition, then captures all selected documents under one
replica lock when Kotlin polls. Kotlin applies each complete publication inside a
Compose snapshot. Canonical nodes use indexed ancestor updates; transient streams
retain UTF-8 byte counts and never enter saved history. Appending an immutable
Compose string still costs proportionally to its current length.

The workspace admits at most four daemon relationships, eight local session
actors, sixteen pending session jobs and thirty-two queued JNI deliveries.
Transfers use the shared bounded coordinator. Image decoding is off the receiver
and UI threads, sampled to 1024 pixels, with eight simultaneous decoded images;
missing or evicted images can be fetched again.

Persistent state includes a private endpoint identity, connection targets,
per-scope drafts, presentation preferences and canonical checkpoints. Checkpoints
are keyed by daemon identity and the full selection. They contain no transient
streams or resumable publication cursor; reopening shows cached history as stale
until the owner supplies a current baseline. Accepted mutations are not replayed
automatically when the connection fails.

The JNI crate is a separate Cargo workspace with its own lockfile. Its filtered
Nix source closure includes `misa-client`, `misa-proto`, `misa-protocol`,
`misa-transport` and `misa-value`; it does not link the session kernel, plugin
runtime or desktop renderers.
