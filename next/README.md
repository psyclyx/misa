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
                                        │  /misa/session/1
        ┌───────────────────────────────┴─────────────────────────────────────┐
        │  a client: subscribes, sends intents, owns the surface               │
        └───┬───────────────┬────────────────┬────────────────────────────────┘
            │               │                │
        misa-tui        misa-web         misa-skia          misa-cli / "misa"
        (cells)      (a server that      (pixels)            (plain text)
                      serves HTML to
                      a browser)
```

The browser never speaks the protocol: `misa-web` is the client, and the browser
gets a document and one stream of replacements.

## Crates

| Crate                                           | What it is                                                          |
| ----------------------------------------------- | ------------------------------------------------------------------- |
| `misa-value`                                    | immutable, structurally shared values and explicit patches          |
| `misa-reframe`                                  | the loop: events, coeffects, effects, subscriptions, transactions   |
| `misa-proto`                                    | the wire: view nodes, intents, subscriptions, framing. No IO        |
| `misa-render`                                   | text measurement, a role-addressed theme, tree → styled lines       |
| `misa-kernel`                                   | facts and capability: the log, the attempt ledger, providers, tools |
| `misa-session`                                  | the agent loop, the view tree, the intent vocabulary                |
| `misa-net`                                      | the protocol state machine, and iroh under it                       |
| `misa-plugin`                                   | the plugin host: a wasm component as handlers and subscriptions     |
| `misa-tui`, `misa-web`, `misa-skia`, `misa-cli` | the four frontends                                                  |

`android/` is the fifth frontend and is not a crate: a Kotlin app over the shared Rust
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
cargo run -p misa-tui -- misa:<endpoint id>:demo
cargo run -p misa-web -- --ticket misa:<endpoint id>:demo --listen 127.0.0.1:8080
cargo run -p misa-cli -- misa:<endpoint id>:demo "say something"
```

The daemon ships a scripted provider, so a session runs end to end with no network,
no account, and no spend. That is the provider every test uses.

`misa-skia`'s raster is behind a feature, because painting needs a Skia that can be linked, which
needs `freetype` and `fontconfig`. That feature is temporary: it exists because Skia is not yet an
input the build provides, and `docs/plan.md` phase 7 builds the pixel frontend with the Skia it
needs, unconditionally — a pixel frontend that cannot paint is not a frontend. Until then the raster
is tested behind the feature:

```sh
cargo test -p misa-skia --features paint
```

The scene — the mapping from a view tree and a theme to positioned runs — is built
and tested without either, which is the interesting half.

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

A plugin presents, too: the tree `view(role, db, window)` returns is placed in the document under a
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
