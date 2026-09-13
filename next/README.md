# misa (rewrite)

A rewrite of misa in Rust, split into a **client**, a **daemon**, and **wasm
plugins**, keeping the previous system's re-frame loop and its
`data → presentation data → renderer` discipline — and finally making the last
stage a real boundary, because a client now lives on the other side of a network.

The design, with its invariants and its open questions, is
[`docs/architecture.md`](docs/architecture.md). Read that first; this file is how
to run it.

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
| `misa-tui`, `misa-web`, `misa-skia`, `misa-cli` | the four frontends                                                  |

`android/` is the fifth frontend and is not a crate: a Kotlin app over the shared Rust
client, linked through a small JNI seam, with its own [`README`](android/README.md). It
draws the same view tree and declares the `mobile` render class, so a session knows the
surface is narrow and that a disclosure has somewhere to live.

## Build and test

```sh
cargo test --workspace                 # the gate
cargo run -p misa-daemon               # prints a ticket
cargo run -p misa-daemon -- login openai-codex   # a subscription, by device code
cargo run -p misa-tui -- misa:<endpoint id>:demo
cargo run -p misa-web -- --ticket misa:<endpoint id>:demo --listen 127.0.0.1:8080
cargo run -p misa-cli -- misa:<endpoint id>:demo "say something"
```

The daemon ships a scripted provider, so a session runs end to end with no network,
no account, and no spend. That is the provider every test uses.

`misa-skia`'s raster is behind a feature, because painting needs a Skia that can be
linked, which needs `freetype` and `fontconfig`:

```sh
cargo test -p misa-skia --features paint
```

The scene — the mapping from a view tree and a theme to positioned runs — is built
and tested without either, which is the interesting half.

## Conventions

- A fact goes in the kernel. A decision goes in the session. An appearance goes in
  a client. If a change needs two of those, it is two changes.
- `misa-proto` has no dependency on IO, and no field that can express a colour,
  a size, or a position.
- A fault is data. Nothing panics on input from a peer or from a model.
- Comments state why, not what. Where a claim can be tested, there is a test that
  pins it, and the architecture document names it.
