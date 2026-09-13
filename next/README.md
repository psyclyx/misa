# misa (rewrite)

A rewrite of misa in Rust, split into a **client**, a **daemon**, and **wasm
plugins**, keeping the previous system's re-frame loop and its
`data → presentation data → renderer` discipline — and finally making the last
stage a real boundary, because a client now lives on the other side of a network.

The design, with its invariants and its open questions, is
[`docs/architecture.md`](docs/architecture.md). Read that first; this file is how
to run it. The work that is left, and the order it is in, is
[`docs/plan.md`](docs/plan.md); [`docs/parity.md`](docs/parity.md) records what of
the previous system is already built.

## The shape

```
        ┌──────────────────────────── misa-daemon ────────────────────────────┐
        │  kernel: the conversation log · the attempt ledger · capability      │
        │  sessions: the agent loop · tools · the view tree                    │
        │  an iroh endpoint, one ALPN per role                                 │
        └───────────────────────────────┬─────────────────────────────────────┘
                                        │  /misa/session/2
        ┌───────────────────────────────┴─────────────────────────────────────┐
        │  a client: subscribes, sends intents, owns the surface               │
        └───┬───────────────┬────────────────┬────────────────────────────────┘
            │               │                │
        misa            misa-web         misa-skia
        (cells/text) (a server that      (pixels)
                      serves HTML to
                      a browser)
```

The browser never speaks the protocol: `misa-web` is the client, and the browser
gets a document, changed HTML subtrees and separate stream updates over SSE.

## Crates

| Crate                               | What it is                                                          |
| ----------------------------------- | ------------------------------------------------------------------- |
| `misa-value`                        | immutable, structurally shared values and explicit patches          |
| `misa-reframe`                      | the loop: events, coeffects, effects, subscriptions, transactions   |
| `misa-proto`                        | the wire: view nodes, intents, subscriptions, framing. No IO        |
| `misa-render`                       | text measurement, a role-addressed theme, tree → styled lines       |
| `misa-kernel`                       | facts and capability: the log, the attempt ledger, providers, tools |
| `misa-session`                      | the agent loop, the view tree, the intent vocabulary                |
| `misa-protocol`                     | both protocol state machines over injected session traits           |
| `misa-transport`                    | iroh, blob transfer, admission and connection drivers               |
| `misa-kit`                          | editor, picker, selection and client memory with injected storage   |
| `misa-plugin`                       | the plugin host: a wasm component as handlers and subscriptions     |
| `misa-tui`, `misa-web`, `misa-skia` | terminal, browser server, and pixel frontends                       |

`android/` is the fourth frontend and is not a crate: a Kotlin app over the shared Rust
client, linked through a small JNI seam, with its own [`README`](android/README.md). It
draws the same view tree the other frontends draw.

## The shell

```sh
cd next && nix-shell              # inside next/ — bare, because ./shell.nix is the whole shell
nix-shell next -A shell           # or from the repository root, next to the root's own shell
```

Entering `next/` with direnv loads it too, after `direnv allow` once: `next/.envrc` is new,
and direnv blocks a new `.envrc` until somebody has read it. Inside `next/` that shell is
what is on the path — the rewrite's toolchain, and not the Zig system's from the root.

One shell, from the pin in `../npins`: the `rustc` and `cargo` this repository is built with
(tooling from a profile is a different toolchain with a different std), `clippy`, `rustfmt`,
`rust-analyzer`, `cargo-nextest`, and — for the plugin host — `wasm-tools`, `wasmtime`,
`wasm-component-ld`, `wabt`, and `lld`. The repository root's shell has the same toolchain
plus the Zig system's, so `cargo` is this repository's rustc wherever you are standing.

## Build and test

```sh
cargo test --workspace                 # the gate
cargo run -p misa-daemon               # prints a ticket
cargo run -p misa-daemon -- login openai-codex   # a subscription, by device code
cargo run -p misa-daemon -- --plugin /tmp/policy.component.wasm   # a policy plugin, in wasm
                                                   # (a client can start the same flow: `/login openai-codex`)
cargo run -p misa-tui --bin misa -- misa:<endpoint id>:demo
cargo run -p misa-web -- --ticket misa:<endpoint id>:demo --listen 127.0.0.1:8080
cargo run -p misa-tui --bin misa -- --print misa:<endpoint id>:demo "say something"
```

Save a received attachment in the terminal with `/save ./photo.png`, or choose an attachment
by its transcript order with `/save 2 ./photo.png`. The command keeps the destination on the
client and refuses to overwrite an existing file. The browser's **Save attachment** button
uses the browser's download location. Both paths ask the session for the attachment it offered,
then fetch the kernel-confirmed bytes. The pixel window offers a local destination dialog for
the same advertised action. Android blob/save and reconnect scenarios are verified on Android 35; final installation against
the packaged ABI remains pending.

Paste text or a desktop clipboard image with Ctrl-V in the terminal. Images become PNG blobs and
remain staged until an explicit prompt send succeeds; Ctrl-Alt-V discards staged attachments.
Alt-Enter inserts a newline, and Ctrl-R searches submission history.

A device login panel offers **Cancel authorization** to stop polling immediately. Dismissing
the panel alone leaves the authorization running.

The daemon ships a scripted provider, so the basic agent loop runs without a provider account or
spend. Provider-specific tests use controlled fixtures; real-endpoint tests exercise transport.

The `misa` binary selects the interactive terminal when stdin and stdout are terminals,
and plain output for pipes. `--print` (or `-p`) forces plain output. `misa-tui` remains an alias.
Skia painting is unconditional; the shell supplies its pinned archive, libraries, and fonts.
Open a native window, or request a headless PNG explicitly:

```sh
cargo run -p misa-skia -- --ticket misa:<endpoint id>:demo
cargo run -p misa-skia -- --view view.json --out frame.png
cargo run -p misa-skia -- --view view.json --window --out frame.png
```

The window supports typed fields, disclosure toggles, tables, meters, images, text selection and
clipboard copy. Stable owner scenes are retained across updates; the pixel command picker remains
unimplemented. The last command saves a snapshot after each window redraw for UI tests.

Build artifacts from the repository root:

```sh
nix-build next -A packages.misa-daemon
nix-build next -A packages.misa
nix-build next -A packages.misa-web
nix-build next -A packages.misa-skia
nix-build next -A packages.misa-guest
nix-build next -A packages.misa-android
nix-build next -A packages.checks
nix-build next -A packages.misa-android.installCheck
```

The guest is installed at `lib/misa/policy-guest.wasm`; the debug-signed APK is at
`share/misa/misa-debug.apk`. The Android install check boots a temporary emulator,
installs that exact APK, and launches its activity; it requires KVM. The checks artifact
runs the Rust workspace, the packaged guest fixture, browser DOM tests, and native Xvfb window
and clipboard checks. These commands describe the available gates; final artifact builds and
installation of the final APK are still pending in `docs/plan.md`.

Both shell entry points derive build inputs from these artifacts. The Android native
libraries use the same pinned Rust version as the desktop builds, with NDK 29 and
16 KiB page alignment. Gradle dependencies are recorded in `android/gradle.lock`.

The repository root also exposes these packages and the previous Zig package as
`misa-legacy`. Its existing modules retain their Zig configuration. The separate
`nixosModules.misa-daemon` module runs the Rust daemon with a private state directory;
configure it through `services.misa`.

## Plugins

A policy plugin is a wasm component: `wit/policy.wit` is the world, and `wit/guest/` is the
smallest thing that implements it. The toolchain builds it in two commands, from this
directory:

```sh
cargo build --manifest-path wit/guest/Cargo.toml --target wasm32-unknown-unknown --release
wasm-tools component new wit/guest/target/wasm32-unknown-unknown/release/policy_guest.wasm \
  -o /tmp/policy.component.wasm
```

The first produces a core module; the second is what makes it a _component_, from the
`component-type` section `wit-bindgen` embeds. To see what came out, and to call it:

```sh
wasm-tools validate --features component-model /tmp/policy.component.wasm
wasm-tools component wit /tmp/policy.component.wasm      # the world it implements
wasmtime run --invoke 'describe()' /tmp/policy.component.wasm
```

`wasm32-unknown-unknown` and not `wasm32-wasip2`: this shell's rustc has std for the first
and not the second, and a plugin imports no WASI — it exports handlers and answers queries,
and `wit-bindgen` + `wasm-tools` do the rest. `wasm-component-ld` is in the shell because
that is what would link a `wasip2` component directly, if the target ever arrives.
The host is `misa-plugin`: it compiles a component, asks it what it is, checks those declarations
against the composition that would run it, and turns its handlers and queries into the loop's
own `Handler`s and `Subscription`s. Its tests are where the two halves meet:

```sh
cargo test -p misa-plugin                          # the conversions and the refusals, no component
cargo test -p misa-plugin --features guest-fixture # the whole path, over a real component
```

The second builds `wit/guest` for `wasm32-unknown-unknown` (needs the shell) and encodes it in
process, so no test needs a `.wasm` in the repository. `MISA_PLUGIN_FIXTURE=/path/to/component`
skips the build and uses one somebody already made, which is what to do when the question is
"is the host wrong, or is the guest?".

A daemon loads them with `--plugin <path>`, once or more. It validates each one against the
session's own list of accepted effects, registers the handlers and subscriptions it declared, and
declares the state roots it asked for (`Ownership::Plugin`) so its patches have somewhere to
land — a plugin that names a root the session already owns is refused at startup rather than
failing inside somebody's transaction later.

A plugin presents, too: the tree `view(role, db)` returns is placed in the document under a
role built from the plugin's id (`plugin.<id>`), with the ids of that subtree prefixed so nothing a
plugin writes can collide with a node the session wrote — so every frontend draws it with no
frontend code at all. Why it is _placed_ rather than merged, and what a plugin may name, are in
`docs/architecture.md` §5.

## Conventions

- A fact goes in the kernel. A decision goes in the session. An appearance goes in
  a client. If a change needs two of those, it is two changes.
- `misa-proto` has no dependency on IO, and no field that can express a colour,
  a size, or a position.
- A fault is data. Nothing panics on input from a peer or from a model.
- Comments state why, not what. Where a claim can be tested, there is a test that
  pins it, and the architecture document names it.
