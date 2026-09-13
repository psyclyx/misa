# Plan

The rewrite implementation plan, in one place: the invariants the design commits to, what to build in
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
   a state, a fact, an image's hash and alt, a message's structure — never local presentation choices. So `graphics`, `native_details`, `RenderClass` and `Capabilities` leave the
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
   `misa-kit` (picker, editor, selection, prefs — storage injected, no `$HOME`). The final dependency gate checks that desktop and Android client builds link neither session nor kernel.
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
| `misa-transport`'s dependency on session and kernel    | `misa-protocol`           | a trait inverts it, so a phone links neither (8)           |
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
- [x] `views::document(db, sections)`: delete the `graphics` read (`image_node` always emits
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

- [x] The in-flight body leaves the tree: a stream per node id with a current value and appends.
      `TextDelta` becomes that stream rather than a duplicate of the tree's content.
- [x] `since` on `Subscribe`: a re-attach inside one session takes the ops it missed; a restart is a
      fresh sync through the canonical view.
- [x] Replace the unbounded per-connection channels with bounded ones; a full queue marks the client
      behind.
- [x] Delete `ClientMsg::Ping`/`Pong` and the ACK question along with it.
- [x] Transport: carry a value of any size (bound the chunk, not the message). Then the `window`
      argument can go, and a client that wants less history is simply rendering less.
- [x] Verify: per token the session writes O(appended bytes) and no tree op; a client that missed ops
      converges by taking the canonical view; a differential test folds the op stream and compares
      with the canonical view.

### 3. The incremental view

- [x] The `Δdb → Δview` mapping in one place, keyed by path prefix; structural ops explicit (append
      child, remove, replace subtree); node content memoised by declared inputs.
- [x] Enumerate the aggregates and give them declared inputs: spend, context meter, queue and notice
      counts, attachments.
- [x] Stable identity for list elements whose front can move (notices, queue) — an index is not an id.
- [x] The audit: rebuild-and-compare on every dispatch in the tests, and a debug assertion for small
      states.
- [x] Complete the client operation gate. Browser SSE emits subtree operations and separate stream
      events; pixels retain owner paint groups with cold-raster parity tests. The terminal retains
      line owners and copies only visible rows during streaming; Android applies indexed canonical
      changes and persists canonical state independently of live streams.
- [x] Verify server audit, operation folding and bounded encoded-change work. See
      `incremental-evidence.md`; pixel layout work is measured separately in `pixel-retained-evidence.md`.

### 4. The crate slice

- [x] `misa-proto` (types and names) · `misa-protocol` (both ends over injected session
      queries, synchronization, intents, completion, info, events and directed replies) · `misa-transport` (iroh, blobs, `serve`, drivers) ·
      `misa-kit` (editor, picker, selection and memory with storage injected).
- [x] Collapse the four attach copies into the protocol's client end.
- [x] Move `admission`/`Roster` out of `misa-session`.
- [x] Documentation follows the completed central crate rename. The final dependency verification
      remains a separate gate below.
- [x] Verify: normal dependency graphs for all four clients contain neither session nor kernel;
      protocol tests use a fake session. Final packaged client checks remain in phase 7.

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

- [x] Artifact derivations exist for the daemon, terminal, web server, pixel window, guest component,
      Android APK and checks, exposed through `next/default.nix` and the root package set.
      Skia is unconditional, with a pinned archive, fonts and native libraries.
- [x] The development shell derives its build inputs from those artifacts, plus development tools.
- [x] Android native libraries, pinned SDK/NDK inputs, offline Gradle dependencies, debug signing and
      an emulator install-check derivation are defined. Native outputs use 16 KiB page alignment.
- [x] A separate Rust daemon NixOS module exposes `services.misa`; legacy Zig modules remain separate.
- [x] Finish artifact integration and housekeeping after the crate rename: locks are refreshed,
      native runtime inputs are declared, and Android builds include only their normal dependency closure.
- [x] Verify the final product derivations, combined checks, guest component, and installation/launch
      of the exact APK on an emulator. All passed; see [`verification.md`](verification.md).

### 8. Client finishing

- [x] A client that keeps what it receives: fetch a blob and write it where a person can find it, and
      cancel a device flow a client started. Both are an intent a session asks the kernel for and an
      answer that is a file or a stop (`architecture.md` §6 item 1).
      Device cancellation and directed save replies are built; terminal file export and browser
      downloads are verified end to end. Pixels also offer a local destination dialog. Android reconnect, blob and save scenarios are verified on Android 35; the final packaged APK also passes emulator installation and launch.
- [x] Android: fetch blobs over `/misa/blob/0` so a picture is a picture; the cursor and the tree are
      data the app persists, so a background check is "attach with `since`".
- [x] Browser: subtree operations and independent streams update the DOM; bounded SSE lag recovers
      canonical HTML plus current streams. Theme, disclosure state and draft belong to page storage.
- [x] Pixels: a native window with editable panel fields, selection and clipboard copy, meters,
      disclosure toggles, wrapped tables and fetched images. Native keyboard, clipboard and
      disclosure interaction are verified under Xvfb; the pixel suite covers local interaction, retained scene identity and cold-raster parity.
- [x] Finish terminal retained presentation and keyboard integration. Bounded request correlation
      keeps input live during completion, upload and save. Images become PNG blobs, survive failed
      submissions, and can be discarded with Ctrl-Alt-V. Alt-Enter interrupts and submits;
      Shift-Enter inserts a newline. Physical rows govern multiline input, selection and cursor placement.
- [x] Verify each frontend's additions, including native X11 and Wayland clipboard transfer.
      Wayland requires compositor data-control support. Final combined artifact verification is recorded below.

### 9. Documentation

- [x] Audit architecture, parity and README claims against the implemented semantic tree, sync,
      protocol boundary, native pixel window, browser updates and attachment flows.
- [x] Remove stale per-client render classes, optional Skia, local-browser runtime instructions and
      historical test totals. Document the single terminal binary and artifact entry points.
- [x] Reconcile terminal and packaged-Android/artifact gates with final integration evidence.
      [`verification.md`](verification.md) records totals from the final runs.

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

- `cargo test --workspace` is the ordinary gate. Guest component fixtures additionally run with
  `misa-plugin/guest-fixture` and the wasm tools. Skia raster tests are ordinary tests; native window
  and clipboard checks run under Xvfb, and browser DOM checks run in Chromium.
- Server differential audits compare incremental state with a rebuild; operation folding and
  stream-offset tests check synchronization. Retained pixel tests compare cold-render pixels and
  count actual semantic layout work. No elapsed-time speedup is inferred from concurrent builds.
- Real-endpoint tests cover reconnect, multiple clients, large chunked views and attachment transfer.
  Browser and terminal file-save tests verify kernel-confirmed bytes; targeted native UI checks
  supplement pure rendering and interaction tests.
- The final workspace, packaged checks, artifact builds and emulator installation all pass.
  [`verification.md`](verification.md) records their scope and final totals.
