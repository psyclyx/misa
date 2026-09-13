# Plan

The rewrite's remaining work, in one place: the invariants the design commits to, what to build in
what order, and what verifies each step. [`architecture.md`](architecture.md) holds the positions,
[`parity.md`](parity.md) holds per-capability status, and this file holds the order.

It replaces the completed plans of the previous system — `ARCHITECTURE-WORK.md` and
`docs/review-followup.md` — which are removed rather than archived. That system's _reference_
documentation (`docs/architecture.md`, `docs/subscriptions.md`, `docs/catalogs.md`,
`docs/services.md`) stays, because that code is still in this tree, and
`docs/history/architecture-audit.md` stays as the archive it is.

## Invariants

A change that violates one of these is a bug in the change, not a trade-off to weigh.

1. **The session emits semantics; the client renders.** The tree carries what a thing _is_ — a role,
   a state, a fact, an image's hash and alt, a message's structure — never how it is drawn or what
   state it is in. So `graphics`, `native_details`, `RenderClass` and `Capabilities` leave the
   protocol; an image node is emitted with its `alt`, and a client that cannot draw it says so in its
   own idiom; a collapsible's expansion is the client's, remembered against the node's `id`.
2. **A client is what it asks for, not what it is.** The hello carries a name and a version; the name
   appears in `/status` and nothing branches on it. The only per-client inputs are requests: the
   subscriptions it holds and the intents it sends. There is no notion of "a phone" or "a terminal"
   anywhere in the session.
3. **The canonical view is the whole session, and a client's state is `(version, tree)`.** Sync is
   one question — "I have version R" — answered with the id-addressed changes since R, or with the
   canonical view. A client that renders only the tail renders the tail; nothing tells the session a
   count. Eviction is safe _because_ the fallback is the canonical view, which is why no ACK, no
   cursor bookkeeping and no protocol keepalive is needed.
4. **The canonical session is a fold over the log; a stream is not canonical.** The log holds settled
   facts, and a session that starts or resumes builds its state and its tree from that fold, once.
   In-flight content — the body of a message still being streamed — is in neither: it is a stream,
   with a current value and appends. A body enters the tree when it enters the log, which is what
   makes "canonical" unambiguous.
5. **The view is maintained incrementally, and the rebuild is the specification.** The tree is
   session state, advanced by a mapping from the transaction's database patches (`Δdb → Δview`) —
   never rebuilt per change, never diffed. Ops are produced where the change is known, are
   self-contained (they carry the node) and are addressed by node id. The rebuild-from-database
   function stays as the init path _and_ as the audit: for every dispatch in the test suite,
   `rebuild(db_after)` must equal `apply(patches, tree_before)`.
6. **A revision's changes are one unit.** Every op of a transaction reaches a client together with
   the version, so no client observes half of one. That is what makes the op stream the fold's own
   steps: any accumulator — the database, the session's tree, a client's tree — converges by
   applying the same ops in order.
7. **Capabilities are named facts, never URLs.** A policy asks the daemon for a _fact_ ("what is left
   on this provider"), and the kernel owns the endpoint, the credential slot and the parsing.
   Nothing in `Intent` can express an effect, a provider parameter or a state path, and nothing in
   the accepted-effects list is a general HTTP request.
8. **The client end of the protocol is one crate, and a client build links no session and no
   kernel.** `misa-proto` (types, and the names both ends must agree on), `misa-protocol` (both ends
   of the state machine, over traits), `misa-transport` (iroh, blobs, the accept loop, the drivers),
   `misa-kit` (picker, editor, selection, prefs — storage injected, no `$HOME`). Today the phone
   links `misa-session` and `misa-kernel` to learn one query name.
9. **Every artifact is a derivation, and the shell is built from the same inputs.** The daemon, the
   terminal, the web frontend, the pixel frontend, the guest component and the phone package are each
   a derivation that declares exactly what it builds against and nothing more, and the development
   shell is composed from those same inputs plus the tools — so a shell and a build cannot drift. A
   frontend is never a feature-gated mode: the pixel frontend is built with the Skia it needs,
   because a pixel frontend that cannot paint is not a frontend.

## What this deletes

The shape of the change is mostly subtraction, so it is worth listing what goes:

| Deleted                                                | Where                     | Why                                                        |
| ------------------------------------------------------ | ------------------------- | ---------------------------------------------------------- |
| `Capabilities`, `RenderClass`, `Capabilities::mobile`  | `misa-proto::wire`        | the session does not act on a client's abilities (1)       |
| `is_narrow`, the `graphics` and `native_details` reads | `misa-session::views`     | the client renders what it can (1)                         |
| `Kind::Collapsible.open`, `Kind::Meter.text`           | `misa-proto::view`        | expansion and formatting are the client's (1)              |
| the view memo's class key, the window argument         | `misa-session`, `views`   | one tree for every client; a count is not semantics (1, 3) |
| `SessionEvent::TextDelta` as a second copy of the body | `misa-session::agent`     | the stream is the body's only representation (4)           |
| `ClientMsg::Ping`/`Pong`                               | `misa-protocol`           | QUIC acks and flow control are the liveness signal (3)     |
| four copies of attach-hello-subscribe                  | tui, cli, web, android    | one client end of the protocol (8)                         |
| `prefs`' `std::fs` and `$HOME`                         | `misa-kit`                | a browser and a phone have their own storage (8)           |
| `misa-net`'s dependency on session and kernel          | `misa-protocol`           | a trait inverts it, so a phone links neither (8)           |
| `admission`/`Roster` in the session crate              | → transport or the daemon | deployment policy is not turn policy (8)                   |

## Phases

Each phase is independently landable, states its verification, and shrinks what follows it.

### 0. Decisions

These are forks that change later work, so they are answered first.

- [x] The incremental view uses structural ops plus memoised node content with declared inputs —
      containers get explicit ops (O(1), nothing to get wrong), content is a memoised build keyed by
      declared inputs (the idiom `Subscription { inputs }` already uses), and only aggregates need
      real delta care.
- [x] The window argument goes after chunked transport lands. Session protocol version 2 carries
      values in 64 KiB chunks with no whole-message ceiling; a real-endpoint test carries a view
      larger than the former 8 MiB limit.
- [x] The per-connection outbound queue holds 64 revision units. A full queue marks the client
      behind; after queued units drain it receives the canonical view before further changes.
- [x] One `misa` is interactive when stdin and stdout are a tty; `--print`/`-p` forces print mode.
      `misa-tui` stays as an alias.
- [x] `gradle2nix` v2 is the second `npins` pin, for its offline `buildGradlePackage`.
- [x] The APK derivation is debug-signed, so the built artifact can be installed on an emulator.

### 1. Semantics: the session stops knowing what a client is

- [x] Delete `Capabilities`, `RenderClass` and the class constructors; the hello carries a name and
      a version.
- [ ] `views::document(db, sections)`: delete the `graphics` read (`image_node` always emits
      `Kind::Image` with its `alt`), the `native_details` read (`Kind::Collapsible { summary }`, body
      as children), the memo's class key (memo by revision, one tree for every client), and the
      capabilities argument to `Section::build`.
- [x] `Kind::Meter` keeps `label`/`value`/`max`; `FieldKind` splits shape (`Inline`, `Block`, `Bool`,
      `Choice`) from policy (`read_only`, `secret`), so a read-only block is expressible.
- [x] Group headers and footers around a run of messages (the parity row asks for a view-tree shape).
- [x] Move `views::VIEW_QUERY` and the completion-source names into `misa-proto` — names both ends
      must agree on belong to the protocol, not to the session.
- [x] The doc comments that argue for what is being deleted — `Capabilities`, `RenderClass`,
      `image_node`, `is_narrow` — and `parity.md`'s line in "Blobs, end to end" that says the
      words-for-an-image decision is made "from the capabilities the client declared" — and every
      other row that describes the capability path (the images row, the panel rows). The _positions_
      are already stated: §1's seams, §2's corollary and §7's numbered questions went in with this
      plan.
- [x] Verify: `cargo test --workspace`, plus a test that two clients attached to one session receive
      byte-identical trees.

### 2. Streaming and sync: content is not the document

- [ ] The in-flight body leaves the tree: a stream per node id with a current value and appends.
      `TextDelta` becomes that stream rather than a duplicate of the tree's content.
- [ ] `since` on `Subscribe`: a re-attach inside one session takes the ops it missed; a restart is a
      fresh sync through the canonical view.
- [ ] Replace the unbounded per-connection channels with bounded ones; a full queue marks the client
      behind.
- [ ] Delete `ClientMsg::Ping`/`Pong` and the ACK question along with it.
- [ ] Transport: carry a value of any size (bound the chunk, not the message). Then the `window`
      argument can go, and a client that wants less history is simply rendering less.
- [ ] Verify: per token the session writes O(appended bytes) and no tree op; a client that missed ops
      converges by taking the canonical view; a differential test folds the op stream and compares
      with the canonical view.

### 3. The incremental view

- [ ] The `Δdb → Δview` mapping in one place, keyed by path prefix; structural ops explicit (append
      child, remove, replace subtree); node content memoised by declared inputs.
- [ ] Enumerate the aggregates and give them declared inputs: spend, context meter, queue and notice
      counts, attachments.
- [ ] Stable identity for list elements whose front can move (notices, queue) — an index is not an id.
- [ ] The audit: rebuild-and-compare on every dispatch in the tests, and a debug assertion for small
      states.
- [ ] Clients apply ops: the terminal appends spans, the browser patches a DOM node, the pixel
      frontend patches a scene node, android applies to its tree.
- [ ] Verify: the audit; the op-stream differential test; bytes per change bounded by the change.

### 4. The crate slice

- [ ] `misa-proto` (types and names) · `misa-protocol` (both ends over a session trait: read, rev,
      intent, complete, info, events, watch_rev) · `misa-transport` (iroh, blobs, `serve`, drivers) ·
      `misa-kit` (today's `misa-client`, renamed, storage injected).
- [ ] Collapse the four attach copies into the protocol's client end.
- [ ] Move `admission`/`Roster` out of `misa-session`.
- [ ] The docs' crate names follow the rename — `misa-client` → `misa-kit`, `misa-net` →
      `misa-protocol` and `misa-transport` — in `architecture.md`, `parity.md`, `README.md` and this
      file.
- [ ] Verify: the android native crate links no session and no kernel; the protocol's tests run over
      a fake session rather than a live runtime.

### 5. Usage

- [x] `kernel.usage` per provider, kernel-side: endpoint, credential slot, and parsing for Claude's
      windows and credits, Codex's `wham/usage` and reset credits, and Kimi scoped to `api_base`.
- [x] The session renders a panel of typed rows, so every surface presents it in its own idiom with
      no client change.
- [x] Port the parity fixtures: `tests/{claude,codex,kimi}-usage.fnl`, `usage-dashboard.fnl`,
      `usage-state.fnl`.
- [x] While here: decide whether `kernel.blob.file` (a path) is something a policy should be able to
      ask for.
- [x] Verify: provider fixtures for the three shapes; the panel asserted on two surfaces.

### 6. The terminal merge

- [x] `misa` becomes the terminal client: interactive when stdin and stdout are a tty, print mode
      otherwise, `--print`/`-p` to force. The print loop moves out of the binary so it can be tested;
      `misa-cli` goes.
- [x] The parity gaps: history search (Ctrl-R), the modal operators (`d`/`c`/`y` with motions, `dd`,
      `o`/`O`), and Alt-Enter.
- [x] Verify: the terminal's own suite plus the print loop's; the hand-checked two-process claim in
      the README with one binary.

### 7. Packaging

- [ ] A derivation per artifact in `next/nix/packages/*`, exposed by `next/default.nix` and
      re-exported through the root `default.nix` (`pkgs.misa-daemon`, …), each declaring exactly the
      inputs it builds against: - `misa-daemon`, `misa` (the terminal) and `misa-web`: the Rust toolchain, and a C compiler
      because `rusqlite` bundles sqlite. Everything else they use (`iroh`, `axum`, `wasmtime`,
      `qrcode`) is pure Rust, so no system sqlite, no OpenSSL, no pkg-config; - `misa-skia`: Skia, freetype and fontconfig, always. The `paint` feature is deleted, the crate
      builds against `skia-safe` unconditionally, and `a_scene_paints_to_a_png` joins the default
      gate; - the guest component: `wasm32-unknown-unknown` and `wasm-tools`/`wit-component`; - `checks`: `cargo test --workspace`, and the fixture tests with the wasm tooling on `PATH`.
- [ ] The shell is composed from those same inputs — the package set's own `nativeBuildInputs` and
      `buildInputs`, not a second list — plus the tools the repository works with (rustfmt, clippy,
      rust-analyzer, cargo-nextest, treefmt, nixfmt, prettier, the wasm tools), so a build and the
      shell cannot drift.
- [ ] Android: the `.so` per ABI through `pkgsCross.<abi>-android` (rustc 1.97.1) and
      `androidenv.androidPkgs` (build-tools 37.0.0, platforms 33–37, NDK 29.0.14206865 — the version
      the README tells people to install by hand), replacing `native/build.sh`; the APK through
      Gradle 9.3.1 with its dependencies available offline (`gradle2nix`, or a fixed-output cache);
      `android_sdk.accept_license`, because the SDK is unfree.
- [ ] The daemon's NixOS module as separate work: the existing module wraps the Zig harness with
      `MISA_CONFIG`, while the Rust daemon takes `--session`, `--data-dir`, `--plugin`,
      `--open`/`--allow` and writes credentials 0600.
- [ ] Housekeeping: `next/android/native/Cargo.lock` has drifted (`misa-client` gained
      dependencies); `views.rs`'s "four roots" comment lists five; `misa-daemon` depends on `clap`
      and parses its arguments by hand, so either the dependency or the parsing goes.
- [ ] Verify: `nix-build next -A packages.<name>` for each from a clean store; the shell can build
      and test the workspace; the APK installs on an emulator.

### 8. Client finishing

- [ ] A client that keeps what it receives: fetch a blob and write it where a person can find it, and
      cancel a device flow a client started. Both are an intent a session asks the kernel for and an
      answer that is a file or a stop (`architecture.md` §6 item 1).
      Device cancellation and directed save replies are built; terminal file export and browser
      downloads are verified end to end. Android and pixel destination selection use the same reply.
- [ ] Android: fetch blobs over `/misa/blob/0` so a picture is a picture; the cursor and the tree are
      data the app persists, so a background check is "attach with `since`".
- [ ] Browser: apply ops to the DOM instead of re-rendering the transcript per revision; its own
      memory (theme, opened nodes, draft) in the page's storage.
- [ ] Pixels: a window; interaction for a panel's fields; selection; and the node kinds the scene does
      not present yet — fields, meters, a collapsible's toggle, tables. Today it draws the text of the
      tree with capture colours.
- [ ] Terminal: the kit's memory shape with the terminal's storage; animations and spinners drawn from
      `State::Streaming`; a pasted binary image through a clipboard capability.
- [ ] Verify: the parity rows move, and each frontend's own suite covers what it gained.

### 9. Documentation

- [ ] `architecture.md`: §6's "what is not built" list as each part of it lands — the change protocol
      in phases 2 and 3, the crate slice in phase 4, the pixel frontend's Skia in phase 7 — and the
      counts as the gate moves. The positions themselves are stated: §1's seams, §2's corollary,
      §3's one tree, §4's "what a change sends", §5's plugin bullets, and §6's limitations.
- [ ] `parity.md`: the rows that move, and the counts. (Its frontend table, its crate-name note and
      its count line were corrected with this plan: the pixel frontend has no picker, no input and no
      collapsible toggle yet, which is phase 8.)
- [ ] `README.md`: one terminal binary and its commands (phase 6), the crate names after the rename
      (phase 4), how to build the packaged outputs (phase 7), and the pixel frontend's build section
      once the `paint` feature is gone (phase 7). A plugin's presentation, the android render class
      and the temporary `paint` feature are already rewritten.

## Parity rows this plan does not cover, and why

Every "next", "partly" and "not built" row in [`parity.md`](parity.md) is either one of the phases
above or one of these — saying which is the difference between a plan and a wish:

- **Subagents.** A child session is a session, so this is composition rather than new machinery, and
  the open question is product-shaped: which state a child inherits from its parent.
- **Session lifecycle (create, list, close).** A daemon opens the one session it is named with, and a
  registry of sessions is a daemon feature until somebody runs more than one.
- **Forking a conversation branch.** The log can express it — a fork is a conversation that starts
  from an entry — and what a branch _is to a client_ has not been decided.

## Verification spine

- `cargo test --workspace` is the gate. Thirteen more tests live behind
  `misa-plugin --features guest-fixture`, and one behind `misa-skia --features paint` until phase 7
  deletes that feature and it joins the gate.
- Two invariants need pinning before the phases that depend on them: the incremental view's audit
  (rebuild against incremental, per dispatch) and the op stream's differential test (fold the ops,
  compare with the canonical view).
- Commit messages record the test count, as the previous ones do, and each phase leaves the gate
  green.
- Three claims stay checked _by hand_, because only two processes can check them: a daemon and `misa`
  over a real endpoint; a web upload and the blob it serves; a name that is not a content hash
  refused before the store is asked. They are named here so they are not mistaken for automated ones.
