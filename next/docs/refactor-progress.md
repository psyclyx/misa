# Refactor implementation evidence

This log records demonstrated work against [refactor-plan.md](refactor-plan.md). A listed command or existing implementation is not a passed gate. Stage 0 is in progress; packaged and emulator gates have not run for this refactor.

## Stage 0 source baseline, 2026-09-16

Before implementation edits, copied `next/` to `/tmp/misa-refactor-baseline-20260916/next/` with `rsync -a --exclude target --exclude .gradle --exclude build`. This includes tracked modifications and untracked source files; no user files were reset. The snapshot directory also contains `base-commit.txt`, `tracked.patch` and `untracked.txt`. These temporary artifacts are local evidence, not repository fixtures.

`cargo metadata --manifest-path next/Cargo.toml --no-deps --format-version 1` identified the actual desktop target directory as `/home/psyc/projects/monorepo/app/misa/next/target`. Baseline tests use the isolated source snapshot and this explicit shared build cache. Cargo serializes builds using its target lock; implementation source edits cannot change the snapshot.

Existing changes fall into these groups:

- Daemon console signal/EOF handling, repeated session creation, local advertisement, admission and persistent client identity.
- Static daemon/session directory wrappers in transport, terminal workspace and browser hub; these are experiments to replace with observations, not the final architecture.
- Terminal composer-owned picker editing, resident candidate delivery, paste and EOF handling.
- Derived queries, provider usage refresh, status/domain projections, configurable indicator composition and plugin indicator declarations. Preserve domain semantics; replace indicator-only ABI and isolated projection caches.
- Terminal retained footer composition, theme/value formatting, and universal terminal component output wired into browser/native rendering. Preserve useful formatting and retained work guarantees; remove the universal terminal output boundary.
- Documentation and Cargo/WIT dependency/fixture changes accompanying those experiments.

## Consumers and gate inventory

| Consumer       | Protocol/application paths                                                                              | Required evidence                                                                                                                                             |
| -------------- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| TUI and print  | `crates/misa-tui/src/{lib,event_loop,remote_requests,workspace,retained,main}.rs`                       | Shared-client scope switching; directed results; responsive composer; retained work; actual PTY Ctrl-C/Ctrl-D and picker/paste; clipboard X11/Wayland.        |
| Browser        | `crates/misa-web/src/{lib,hub,updates}.rs`, browser JS and `tests/browser.sh`                           | Atomic DOM transactions; per-tab instances/drafts; SSE lag recovery; bounded handle cleanup; Chromium DOM tests.                                              |
| Native window  | `crates/misa-skia/src/{connection,app,window}.rs`, `tests/window.sh`                                    | Incremental paint/layout; asynchronous assets; native actions; Xvfb/keyboard/clipboard checks plus remote scope switching.                                    |
| Android JNI    | `android/native/src/{lib,delivery,snapshot,files}.rs`                                                   | Standalone Cargo tests; snapshot identity/resume; history-independent incremental JNI bytes and no per-token persistence; bounded command/transfer ownership. |
| Android Kotlin | `android/app/src/main/java/org/misa/app/{Native,MisaViewModel,Wire,ViewTree,LiveStreams,Transcript}.kt` | Complete transaction callbacks; per-instance state; no late-handle resurrection; indexed identity and UTF-8 offsets; real JNI/iroh instrumentation.           |
| Plugin         | `crates/misa-plugin`, `wit/policy.wit`, `wit/guest`                                                     | Real guest fixture, declaration validation, faults and capability variants.                                                                                   |
| Packaging      | `nix/packages/checks.nix`, `nix/android.nix`, both Cargo lockfiles                                      | Desktop checks package, all desktop artifacts, guest, three Android ABIs/APK and emulator execution.                                                          |

Keep the new client dependency graph free of kernel/session/plugin runtime and desktop rendering dependencies. Android is a separate Cargo workspace: desktop workspace tests do not cover it. New client dependencies must enter Android's filtered `nativeCrates` source closure and independent lockfile. Preserve pure reusable interaction algorithms; move terminal-specific selection and local application preferences to their owners.

## Commands and results

### Desktop baseline

Running against frozen source:

```sh
cargo test --manifest-path /tmp/misa-refactor-baseline-20260916/next/Cargo.toml --target-dir /home/psyc/projects/monorepo/app/misa/next/target --workspace
```

Log: `/tmp/misa-refactor-baseline-20260916/desktop-tests.log`. Passed, exit 0: 588 tests passed, 1 ignored across workspace/unit/doc targets. The ignored test is `clipboard::tests::desktop_image_and_text_roundtrip`, which requires a desktop display. Baseline commit: `8132117bc95704fdcf7e4e0c4428c0584ddfeb2c`, plus the captured working tree changes.

### Android native baseline

```sh
cargo test --manifest-path /tmp/misa-refactor-baseline-20260916/next/android/native/Cargo.toml --target-dir /home/psyc/projects/monorepo/app/misa/next/android/native/target
```

Log: `/tmp/misa-refactor-baseline-20260916/android-tests.log`. Passed, exit 0: 7 tests, covering verified files, persistent identity, bounded uploads, canonical snapshot persistence/identity and incremental delivery. This is a host test, not an Android cross-build or emulator run.

### Process baseline

Built frozen sources with:

```sh
cargo build --manifest-path /tmp/misa-refactor-baseline-20260916/next/Cargo.toml --target-dir /home/psyc/projects/monorepo/app/misa/next/target -p misa-daemon -p misa-tui
```

Build passed; log `process-build.log` under the evidence directory. Copied the resulting `misa` and `misa-daemon` into the snapshot's `next/target/debug` before running processes. `binaries.sha256` records those exact copies, preventing subsequent implementation builds from changing the tested executables.

The existing `/tmp/misa-integration.py` was copied to `pty-check.py` in the evidence directory, pointed at those binaries, and corrected to allocate a 120-column/36-row PTY. Its original zero-sized PTY could not render the expected directory; that diagnostic is `pty-zero-size-harness.log`, not evidence of a renderer regression.

The initial run exposed an implementation failure with a long runtime path: `nix-shell` supplies a long temporary root, and placing the local runtime directory there caused daemon startup to exit with `path must be shorter than SUN_LEN`. Shortening the daemon filename was insufficient for arbitrary configured runtime paths. Failure evidence: `pty-long-runtime-path.log`. The ordinary-path baseline rerun explicitly used `TMPDIR=/tmp`; the subsequent fix and long-path verification are recorded below.

```sh
nix-shell -p python3 --run 'TMPDIR=/tmp python3 /tmp/misa-refactor-baseline-20260916/pty-check.py'
nix-shell -p python3 --run 'TMPDIR=/tmp python3 /tmp/misa-refactor-baseline-20260916/noninteractive-check.py'
```

PTY check passed, exit 0: local pairing and print response; two daemon discovery; directory without an attached session; terminal empty-input Ctrl-D; foreground daemon Ctrl-C without another newline; foreground daemon EOF; discovery socket cleanup. Log: `pty-check.log`. Noninteractive check passed: `/dev/null` stdin leaves the daemon alive; SIGINT exits with code 0 and removes its discovery socket. Log: `noninteractive-check.log`. The scripts stop only their own child processes and use isolated state directories.

### Remaining baseline gates

- Picker/paste behavior has unit evidence from the workspace suite, not a new PTY scenario.
- Browser DOM, native window, clipboard, real WASM guest, packaged builds and Android emulator: not run in Stage 0 yet. No heavy package builds requested for this baseline task.

The Android emulator gate requires running the built wrapper, not merely building it:

```sh
nix-build next -A packages.misa-android.installCheck -o result-android-check
result-android-check/bin/misa-android-check
```

## Local socket path correction

`transport/local.rs` now resolves one directory consistently for advertise, discover and pair. If the configured directory plus socket filename exceeds the conservative 103-byte Unix pathname budget, it uses `/tmp/misa-<effective-uid>-<requested-directory-hash>`. The fallback does not depend on potentially long `TMPDIR`. Different requested namespaces remain distinct.

All three operations require an actual directory owned by the effective user with mode 0700, rejecting symlinks and public directories. Both socket peers verify the connecting process UID; pairing still verifies the full daemon identity before sending the client identity. The Unix-only `libc` dependency supplies the effective UID. Cargo metadata updated the dependency edges in desktop/Android lockfiles; package versions did not change. The path bound accommodates Linux and BSD/macOS pathname sizes; this run verifies Linux, not other operating systems.

Four focused tests passed with `cargo test --manifest-path next/Cargo.toml -p misa-transport local::tests`: long-path fallback discovery/pairing and namespace separation; private-directory/symlink/owner checks; full identity refusal before client-key disclosure; multiple clients without consuming a remote invitation. Log: `/tmp/misa-local-fix-tests.log`.

To isolate the correction from simultaneous protocol edits, copied the frozen Stage 0 source to `/tmp/misa-local-fix/source`, then overlaid only `transport/src/local.rs` and its Cargo manifest. Built fresh binaries:

```sh
cargo build --manifest-path /tmp/misa-local-fix/source/Cargo.toml --target-dir /home/psyc/projects/monorepo/app/misa/next/target -p misa-daemon -p misa-tui
nix-shell -p python3 --run 'python3 /tmp/misa-local-fix/pty-check.py'
```

Both passed. Binaries were copied into `/tmp/misa-local-fix/target/debug` before execution; hashes are in `/tmp/misa-local-fix/binaries.sha256`. Unlike the ordinary-path baseline, this harness retained Nix's long temporary root. Log `/tmp/misa-local-fix/pty-long-runtime.log` shows a runtime root under `/tmp/nix-shell-…/build-top/misa-check-…`, with both daemon sockets in `/tmp/misa-1000-3e8e85685198a3ad`. It verifies discovery/pairing, print response, independent two-daemon directory, terminal EOF, daemon Ctrl-C/EOF and socket cleanup. The harness removes only its own empty fallback directory and child processes.

Broader isolated reruns passed: `cargo test --manifest-path /tmp/misa-local-fix/source/Cargo.toml --target-dir /home/psyc/projects/monorepo/app/misa/next/target -p misa-transport` (49 tests) and `cargo test --manifest-path /tmp/misa-local-fix/source/android/native/Cargo.toml --target-dir /home/psyc/projects/monorepo/app/misa/next/android/native/target` (7 tests). Logs: `/tmp/misa-local-fix/transport-tests.log` and `/tmp/misa-local-fix/android-tests.log`. No cross-platform/package verification is implied by the Linux PTY result.

## Shared client coordination core

Added `misa-client` with a connection-independent, per-daemon coordinator. It delegates replica application/checkpoints and render invalidations to `misa-protocol::observation::Replica`; allocates nonreused observation/invocation IDs; bounds observation and pending-call counts; correlates results once; distinguishes timeout/disconnect uncertainty from domain cancellation; and reconnects observations without replaying writes. Observation generations advance independently of driver connection generations: replacing a failed observation rejects queued snapshots from the previous watch even within the same live connection. Explicit cancellation releases observation interest, while local invocation abandonment never cancels domain work. Outgoing values map directly to scoped protocol envelopes. Transport integration, transfer coordination and frontend migration remain incomplete.

`cargo test --manifest-path next/Cargo.toml -p misa-client` passed 5 lifecycle tests covering independent scopes, capacity release, stale handles, one recovery request per gap, checkpoint binding, closed-owner lifetime, replay-free reconnect, late replies, deadlines and malformed-result containment. Log: `/tmp/misa-client-tests.log`.

Subsequently added finite reads using `scoped::Read`/`ReadReply`. A read shares the pending-request budget, returns a coherent immutable result validated by the existing replica implementation, and leaves no retained subscription. Timeout, local abandonment and disconnect release its pending entry; late/foreign-generation replies are ignored. `Settled` returns reads separately from invocation outcomes because a read timeout is not an uncertain mutation. All 8 client tests passed, including finite-read correlation/budget/reconnect and the independent-generation queued-snapshot regression (`/tmp/misa-client-finite-tests.log`).

Added the dependency to Android's standalone native manifest/lock and filtered Nix source closure. `cargo check --manifest-path next/android/native/Cargo.toml` passed on the Linux host (`/tmp/misa-client-android-check.log`); this does not claim Android ABI compilation. `cargo tree --manifest-path next/Cargo.toml -p misa-client --edges normal --prefix none` confirms no kernel, session, Wasmtime or rendering dependency (`/tmp/misa-client-dependencies.txt`).

## Scoped client transport slice

Added low-level `transport/scoped_client.rs` and shared `scoped_io.rs` on the preparatory `/misa/scoped/3` ALPN. Greeting validation binds advertised daemon identity to the authenticated iroh peer and checks the daemon scope/version. Dedicated reader/writer tasks own framing and partial writes. Logical messages have a 64 MiB bound in addition to 64 KiB chunks; serialization is checked before extending the encoding buffer. This does not eliminate control-stream head-of-line blocking; independent output lanes remain a cutover gate.

The high-level owner is `misa-client/src/driver.rs`, depending on low-level transport, not the reverse. It consumes publications into shared replicas regardless of UI notification lag. Per-observation watch notifications coalesce; `inspect` presents the current replica and latest sequence/invalidation under one short lock. A surface can apply a contiguous invalidation or rebuild after a notification gap. Directed reads/invocations use separate bounded pending oneshot routes. Dropping an observation releases interest; closed-receiver scanning ensures a full cancellation queue cannot leak a lease. Failed remote cancellation enqueue closes the connection rather than silently retaining server interest.

`cargo test --manifest-path next/Cargo.toml -p misa-client` passed 10 tests (`/tmp/misa-driver-tests.log`), including actual iroh connections to the scoped owner handler: 100 updates without consuming UI notifications do not block directed commands, finite reads or replica advancement; dropping the lease releases the server observation; a greeting naming another daemon is refused. Android native host `cargo check` passed (`/tmp/misa-driver-android.log`), and the normal client dependency graph remains free of kernel/session/rendering/Wasmtime (`/tmp/misa-driver-dependencies.txt`). Automatic reconnect, transfer coordination and frontend cutover are not implemented by this slice.

### Subsequent lane and reconnect implementation

The scoped transport now separates persistent ordered observation lanes and one-shot read-result lanes from the authenticated control stream. Lane headers identify existing handles/request IDs; payloads retain the same publication/result vocabulary. A bounded frame-length prefix allows the receiver to reserve aggregate bytes before allocating a payload. Senders count serialization before reserving actual encoded bytes. Data and control use independent budgets (128 MiB and 8 MiB respectively), so a blocked large observation does not monopolize command results. The client accepts data lanes concurrently and keeps byte permits through its delivery queue. A failed observation lane replaces only that observation under a new generation; stale lane closures/frames cannot affect the replacement. Server-side lane fairness evidence is recorded by the scoped server tests.

The shared client driver automatically retries a lost authenticated connection up to six times with capped backoff while continuing local request rejection, deadline handling and lease cleanup. It resumes retained observations, never invocations or finite reads. `status()` exposes phase, link generation, last connection error and the current authenticated daemon scope; a new owner incarnation remains visible and old scope selections are closed rather than silently rewritten. `disconnect()` stops the socket/retry task and settles pending work even when the ordinary command queue is full. `Observation::watch()` supports additional local notification consumers without moving the lease; `daemon_identity()` binds render-cache identity across daemon switches.

The real reconnect test executes a mutation whose result is deliberately withheld, cuts the connection, observes an indeterminate result, and verifies reconnection does not execute it again. It covers unchanged and replaced owner incarnations, retained observation recovery, and explicit disconnect disabling retry. The client suite passed 13 tests after this work (`/tmp/misa-reconnect-tests.log`). `cargo test --manifest-path next/Cargo.toml -p misa-transport scoped_` passed 5 tests, including the real unread-large-observation fairness check (`/tmp/misa-lanes-tests.log`); Android native host `cargo check` passed (`/tmp/misa-reconnect-android.log`). Transfer coordination and frontend cutover remain outstanding.

### Shared daemon registry lifecycle

`misa-client::daemons` now owns a stable directory watch separately from its replaceable bootstrap observation. A newly authenticated Welcome with a new daemon incarnation refreshes only that bootstrap selection; independently held old scopes remain closed. Directory snapshots carry their scope and freshness. Replacement retires the old lease before requesting another slot and retains explicitly stale data during transient replacement failure. Cancellation of the last unfinished connect future releases its registry capacity; concurrent waiters still share one attempt.

Validation: `cd next && cargo test -p misa-client` passed all 16 tests, log `/tmp/misa-registry-tests.log`. Three new actual iroh tests cover concurrent same-peer deduplication, two daemon directories with identical session titles, explicit disconnect isolation, restart bootstrap refresh through an existing watch while an independent old scope closes, and cancelled-connect capacity release. No new normal server dependencies were added.

## Client preparation validation and cancelled transfers

Shared `Interaction::load` now validates source and shortcut identities, query contracts, command targets, and argument/source references before exposing remote preparation metadata. Two targeted client tests pass: malformed references are rejected, and read-only shortcut preparation remains a finite read rather than becoming a mutation.

The reusable blob store now removes a connection from its cache for the duration of an exchange and returns it only after a complete successful response. Cancelling a request during a partial frame therefore drops the stream; the next fetch opens a fresh connection. A real iroh regression sends all but the last response byte, cancels that fetch, and verifies a second hash is fetched on a new stream. `cargo test --manifest-path next/Cargo.toml -p misa-transport blob::tests --quiet` passed all seven tests. This fixes transfer cancellation framing; bounded shared transfer coordination and frontend integration remain separate outstanding work.

### Reconnect pressure and observation capacity

The shared client now restores observation interests by asynchronously reserving one transport writer slot at a time. Its bounded interest FIFO preserves Observe/Cancel ordering; new observations receive transient `busy` while older handles drain, preventing a newer handle ID from overtaking a resumed one. Reads, invocation results, cancellation, and explicit disconnect continue through the owner loop. No mutations replay, and the transport writer queue remains 64 items.

A real socket regression opens 112 current observations, reconnects once, cancels 16 during restoration, checks all remaining 96 replicas and server interests, and exercises directed reads. This uncovered two separate limits: server default 64 versus client default 128, and QUIC's default incoming stream credit below the application's lane bound. Server/client now share a default 128-observation limit including bootstrap interests; scoped connections explicitly grant the existing 160-reader bound in QUIC stream credit. The regression passes with connection generation exactly 2.

Validation: `cd next && cargo test -p misa-client -p misa-transport` yielded all 19 client tests passing and 54/55 transport tests passing. Log `/tmp/misa-resubscribe-tests.log`. The sole failure is existing legacy `server::tests::a_large_canonical_view_crosses_a_real_endpoint`: its pre-attach expected view lacks a subsequently published notices section (`notices.count=1`); the huge model payload is unchanged. Root notified. Focused regression log: `/tmp/misa-many-interests-tests.log`.

## Skia scoped PNG export

The production PNG export path now uses shared daemon relationships, observed directory entries with exact session incarnation, installed presentation selection, and the shared replica. It no longer attaches a legacy client or owns a ClientView/recovery loop. Its endpoint persists the misa-skia client identity. Full document materialization is explicit and limited to requested raster frames; the shared materializer includes the live overlay without mutating canonical state. Native window migration remains outstanding.

`cargo check --manifest-path next/Cargo.toml -p misa-skia --quiet`, the document reader/materialization test, and fresh daemon/Skia builds passed. A real-process Python harness under `nix-shell -p python3` started an isolated scripted daemon with `--no-relay`, local automatic pairing, temporary XDG runtime/state roots, and ran Skia PNG export twice. Both produced a valid 32622-byte PNG and reused the same persisted 32-byte client identity; the harness stopped its own daemon via SIGINT. An earlier run against the prior binary passed export but correctly failed the identity assertion; only the fresh-build rerun proves both behaviors.

Verified binary SHA256 values: daemon `2546cdbeb1d580db02d60543ee077292dd872025eeca92bb417a2dc684c883ae`; Skia `ec491defa42c63197beb323d90edc976d11f1b04a376e43ffc9411fe284d8e0e`. This is headless raster evidence, not native window/display or packaged artifact verification.

Follow-up validation is now green: `cd next && cargo test -p misa-client -p misa-transport` passes all 21 client and all 55 transport tests, log `/tmp/misa-transfer-and-recovery-tests.log`. The legacy large-view test now validates the received tree and compares its immutable oversized model subtree exactly, retaining corruption/chunking coverage without racing unrelated notices.

### Shared transfer admission

Daemon relationships now expose `misa-client::transfers::Transfers` through `Daemon.blobs`, preserving asynchronous `get`/`share` calls. Shared count/byte admission and network deadlines bound transfer work across surfaces; offered reference lengths are validated by `download`. `decode` moves CPU work to a blocking worker whose admission remains held even after its awaiting caller cancels. Rendered/decoded output caches remain surface-owned and must impose their own output bounds. Explicit daemon disconnect/drop closes the coordinator, cancels outstanding asynchronous socket work, and refuses new transfers. No blob work is added to the scoped publication receive loop.

Two actual blob-protocol tests verify reference mismatch detection, content-addressed upload reuse, cancellation retaining blocking-decoder admission until completion, repeated network timeout releasing count/byte budgets, and explicit close cancelling/refusing work. Included in the successful client/transport run above. Existing scoped TUI upload calls require no signature changes and remain in bounded pending tasks.

## Native retained document application and local reports

Skia App now applies shared document updates through its retained tree/live caches. A composed update applies canonical changes and live retirement before returning to the window; unchanged history retains its paint groups. A new 10/1000-owner test verifies atomic settlement, bounded changed-node layout, retained allocation identity, and pixel equality with a cold render.

Read-only reports are local App state with scroll, copy, and dismissal. They do not enter the replica or overwrite composer drafts; report wrapping is cached until width changes. Rejected/indeterminate prompt text returns to an empty composer or remains in a separate copyable recovery report when newer typing exists. Tests verify dismissal emits no domain action, newer text survives, and canonical content is unchanged.

`cargo test --manifest-path next/Cargo.toml -p misa-skia --lib --quiet` passes all22 tests after these changes. Native transport/window event cutover is still being implemented; this result alone does not claim a working scoped native window.

## Browser transaction delivery

SSE now emits each Region publication as one `transaction` event, including snapshot plus live reset. Browser application consumes every member synchronously before paint or MutationObserver delivery. Invalid DOM application hides the invalid cache and opens a fresh SSE subscription; only a replacement snapshot can make it current again. Composer draft, focus, and selection survive this recovery. This is the browser delivery boundary; the legacy web network owner still needs conversion to shared scoped observations.

The Chromium DOM fixture now verifies atomic canonical/live settlement as observed by MutationObserver, invalid-cache visibility, refusal to revive it with a delta, and coherent replacement preserving composer state. `bash next/crates/misa-web/tests/browser.sh` passes. Added a Rust batch-boundary test showing initial/replacement tree and live state remain in their respective publication batches. Full web library tests pass29 tests. No native client/backend migration completion is implied by this browser delivery result.

### Scoped native window

Skia's production native `connection.rs` now resolves the ticket through shared daemon relationships/directory, loads the installed Interface and Interaction catalogs, and observes the chosen session incarnation. It no longer attaches a legacy transport client or owns a ClientView recovery loop. One coalesced window wakeup captures the latest shared replica with `document::Reader`; the window calls `App::observed` directly. A second independent document consumer discovers image references from resets and ordered inserted/replaced subtrees.

Commands, operation-result observations, authoritative `session.attachment.resolve` saves, downloads, and blocking filesystem writes run as bounded independent tasks. Completed data/read commands open the local structured report; rejected/indeterminate prompt text is restored without overwriting newer typing. Image fetch/decode stays off observation delivery, uses shared Transfers and decoder allocation/dimension limits, and has at most two decoded image events awaiting the window. The renderer cache is limited to32MiB; eviction releases retained scene Arcs and exposes an explicit local Load image action. Dropped image owners are pruned and late decodes cannot repopulate removed owners.

Validation: `cd next && cargo test -p misa-skia` passes all23 tests, `/tmp/misa-skia-scoped-tests.log`, including cache release/reload/removal behavior. Fresh daemon/Skia binaries passed an isolated Xvfb actual-ticket smoke: local pairing, scripted prompt/streamed turn, `/status` structured local report, report clipboard, Escape dismissal, clean daemon SIGINT. Harness `/tmp/misa-skia-native-smoke.sh`, log `/tmp/misa-skia-native-smoke.log`, screenshots `/tmp/misa-skia-native-Gekr22/{initial,prompt,report}.png`. The existing native window keyboard/clipboard/disclosure/picker fixture also passed, `/tmp/misa-skia-window-fixture.log`.

This proves the ticket-selected native running path. Multi-daemon/session selection controls and local request dialogs are still outstanding native integration work; a directory-backed relationship does not by itself constitute those UI features.

## Web backend scoped connection cutover

Production web session instances now use shared Daemons, Interface/Interaction, document Reader, and bounded Transfers. The former attached connection task, unbounded intent channel, pending download correlation table, and ClientView cursor are removed. HTTP actions use the shared invocation path; attachment downloads resolve the authorized node through session.attachment.resolve. Read-only shortcuts render separate report responses. Each Remote aborts its observation task on drop. Region retains only derived indexed content/live caches for HTML and SSE snapshots; a failed cache update cannot expose a partially applied snapshot. Connection freshness is displayed separately from authoritative content.

A real scoped-only iroh handler regression connects two web presentation instances, rejects an uninstalled command, invokes a scripted prompt, observes its result in both instances, and verifies pending attachments remain instance-local and one instance can be dropped independently. `cargo test --manifest-path next/Cargo.toml -p misa-web --lib --quiet` passes30 tests; Chromium browser checks pass after this integration.

Remaining web work includes replacing cookie-wide session selection with tab-instance routing, bounded instance cleanup, reactive directory UX, request forms, provider/model preparation UI, and preserving submitted drafts through rejected/indeterminate HTTP outcomes. The current cutover does not claim those interaction requirements are complete.

Final native smoke was repeated after rebuilding the completed image-cache and queue-rejection changes: passed again, artifacts `/tmp/misa-skia-native-4dZdMM`, with the same exact harness/log paths above.

## Browser presentation routing and lifetimes

Selection now creates a distinct `/view/{instance}/` URL. Resource URLs, forms, SSE, and redirects are relative to that instance; dispatch ignores cookies and routes only by URL. Browser preference keys include daemon identity, scope incarnation, and presentation instance. An explicit Close presentation action aborts only its observation task, leaving domain operations and other views intact.

The hub bounds retained presentation instances to64. Idle instances expire after30 minutes; a periodic sweep releases unused observation owners. Active SSE responses retain their exact Remote lease, so a live tab is not evicted by that idle policy. A real scoped-server regression opens two session views, asserts independent URLs and no selection cookie, sends a conflicting cookie to each URL and verifies correct session routing, then proves an active stream keeps only its own expired instance alive. Full web31 tests and Chromium checks pass.

Copying a presentation URL still addresses the same retained instance; automatic independent tab duplication/recovery UX, provider/model controls, private workflow forms, and selected optional presentation placement remain outstanding. Status/plugin documents are now independent catalog presentations on the server; web must explicitly place their selected variants in the next presentation-composition step.

## Browser submission recovery and directed outcomes

Composer submission now uses the declared shortcut parser and shared interaction preparation. Browser submissions retain recovery text before sending, suppress duplicate pending form submissions, and clear only an unchanged draft after acknowledgement. Newer typing survives late replies. Rejection and lost-reply recovery copies remain locally available without automatic replay. Read-only reports open dismissible local dialogs. Submission notices are separate from connection freshness.

The Chromium fixture passes delayed acknowledgement, newer typing, duplicate suppression, rejection, report dismissal and uncertain response scenarios. The scoped HTTP integration verifies an ordinary composer prompt and local status report. The HTTP adapter now preserves Completed/Accepted/Rejected/Indeterminate outcomes instead of collapsing acceptance to null and uncertainty to rejection; the regression verifies that a prompt returns its exact scoped operation reference. Web library tests pass31 after adding this assertion. Browser operation-result presentation and selected optional documents remain outstanding; exposing an operation reference alone does not complete that workflow.

## Native HTML status rendering

Removed the web renderer's terminal-component shortcut. It flattened status children into terminal lines, losing stable child DOM identities, semantic fact markup and action forms. Status now follows the ordinary semantic node renderer; CSS controls wrapping and placement without terminal column measurement. A regression checks preserved child identities, formatted money data and the usage action's original domain node identity. Production web library check, all32 web library tests and Chromium fixture pass. Multi-document selection and coherent delivery remain separate outstanding integration work.

### Presentation DOM identity

The HTML renderer now accepts a local DOM prefix independently of domain node identities. It propagates through ordinary children and list items while command/download forms retain the original node target. Local report responses allocate distinct prefixes so simultaneous reports do not collide with each other. The two-document renderer regression passes with the full33-test web suite before report call-site integration; selected document caches and atomic multi-document SSE delivery still need integration.

## Coherent selected documents in web

Production web now resolves conversation/status defaults through shared composition preferences before observing content. Optional plugin presentations remain unselected. All selected document readers are captured under one replica lock, then their retained HTML caches are updated before publishing one SSE transaction. Documents have separate DOM prefixes and live-stream maps; action node identities remain authoritative domain identities. Incremental changes render only changed subtrees.

A failed member update publishes nothing and hides the invalid aggregate cache. Repair requires replacement snapshots for every previously selected document. Browser recovery likewise refuses a partial replacement. Web34 tests pass, including coherent cache publication/failure/repair and the real scoped HTTP session path. Chromium checks pass multi-document MutationObserver coherence, same-ID independent streams, invalid-composition hiding and full replacement recovery. Runtime presentation preference controls, dynamic selection replacement and private workflow forms remain outstanding web work.

## Open transaction-staging regression

Added and ran `journal_staging_preserves_sequential_handler_reads`. It deterministically fails: two ordered increment handlers produce replayed value1 instead of2. `Confined::handle` removes each handler's patches before the next handler sees the transaction's working state. The same staging arrangement also permits a later command to compute from old state before acknowledgement. This is an active correctness failure, not a completed gate. Staging must move to the complete transaction boundary; a per-handler busy flag would still violate sequential composition. Root owns the finalization/admission fix while daemon lifecycle and client work continue independently.

### Transaction-finalization repair

That regression now passes. Confined handlers retain their working patches throughout dispatch; owner finalization aggregates their journal decision and withholds plugin-root publication until acknowledgement. Competing mutations receive an explicit busy rejection. Exact decision matching prevents duplicate acknowledgements and stale failures from applying or releasing another transaction. A matching append failure releases admission and emits an error notice. Removed the obsolete per-handler patch extraction API.

The reframe29/session136 suites pass; the subsequently extended regression also passes sequential reads, unpublished pending state, competing admission, duplicate ack, stale failure, exact failure and renewed admission. Remaining audit work: automatic kernel events with mutating plugin listeners need bounded deferral instead of being lost on busy rejection; transactions combining authoritative replay patches and new plugin writes need precise staged-patch attribution. These broader cases are not claimed complete by the passing direct-command regression.

### Exact deferred-write attribution

Transactions now retain exact positions of writes marked for owner finalization. Plugin staging removes only those candidate writes, preserving acknowledged/replayed patches in the same dispatch. Admission checks the transaction's resulting pending-decision state, allowing an acknowledgement listener to stage a subsequent decision after resolving the old one. The new regression proves that first acknowledgement publishes value1 while the listener's value2 remains pending, and the next acknowledgement publishes value2. A reframe regression requires rollback of writes and effects if deferred writes lack finalization. Full reframe30/session137 tests pass. Automatic kernel-event deferral remains outstanding.

### Bounded automatic-report deferral

Automatic kernel reports rejected by pending plugin persistence are now retained in an owner-local queue. Journal replies continue through dispatch and release queued work after the pending decision resolves. Handlers run outside queue locks; a wake counter prevents an acknowledgement racing a blocked retry from leaving the queue parked. Retention is bounded to128 reports and8MiB of conservatively counted value data. Capacity exhaustion explicitly closes the owner rather than discarding an authoritative report and continuing with incomplete state. Shutdown releases the queue.

The runtime regression holds journal acknowledgement, verifies the automatic increment remains queued with unpublished state, supplies the acknowledgement, and verifies the queued write is evaluated and then published by its own acknowledgement. It also exercises count overflow, explicit closure and queue cleanup. All138 session library tests pass. This closes the previously recorded automatic-report loss case; it is not a claim that the full refactor's client, packaging or deletion gates are complete.

## Web presentation controls and replacement

Each web instance now exposes catalog-derived automatic/hidden/supported-variant controls. Conversation stays visible as the primary input surface. A single admitted reconfiguration opens the replacement observation and waits for a current snapshot while the existing selection remains usable. The instance actor replaces all selected caches and preferences together; unsupported choices or invalid replacement snapshots preserve the old selection. SSE carries the replacement selection alongside its complete document transaction, and browser removal releases deselected live-stream caches.

The real scoped-server HTTP regression verifies bounded concurrent admission, invalid-variant rejection, hide/show, retained conversation, and a second view's independent status visibility. Full34 web tests pass. The Chromium fixture verifies removal of deselected documents without removing the selected conversation. Preferences currently live with the retained instance; cross-process persistence, copied-tab independence, provider preparation and private workflow forms remain outstanding web requirements.

### Browser-owned presentation defaults

Web pages now expose a stable daemon/session preference key separately from incarnation/instance draft keys. Browser storage retains acknowledged presentation choices as defaults for future instances. Restoring defaults and interactive changes use the same bounded, serialized local-selection endpoint; they do not broadcast preference changes into other live views. Controls revert to the last confirmed choice on failure. Missing saved variants produce a local diagnostic instead of silently selecting an unsupported variant. Storage denial leaves the current instance usable.

Chromium checks pass restoration across instance changes, acknowledged persistence, duplicate pending suppression and rejected-choice restoration. Production web compilation passes. Restoration currently follows initial default selection; optimizing initial selection to consume browser defaults before content observation remains a refinement, alongside copied-tab isolation, provider preparation and workflow forms.

## Copied browser presentation isolation

The scripted browser claims a presentation before enabling controls or starting observation delivery. A second document loading an already claimed URL receives a new presentation URL with independent observation, preferences and attachments. Initial draft/recovery memory is copied to its new instance key; later edits are independent. Claims remain reserved across SSE reconnects, avoiding a transient disconnect handing the same instance to another tab. Copying a stale session incarnation is rejected rather than rebinding it silently.

Reload also creates a new presentation instance. Old instances retain the existing bounded idle-expiry policy; the daemon directory now lists retained presentations with explicit close controls. This is a deliberate current lifetime behavior, not a claim of automatic reuse on reload. Without JavaScript, users still open independent instances through the session chooser; copied-URL automatic claiming requires script.

The real HTTP regression verifies two claims yield distinct instances of the same exact scope with independent attachment ownership. Chromium verifies interaction gating, redirect, initial draft copy and later storage isolation. Provider preparation, private workflows and initial preference-before-observation remain open web work.

## Web private workflows and accepted work

Each web instance now observes operation/request summaries independently of its selected documents. Every invocation reserves bounded tracking capacity before sending; accepted references use the shared operation tracker, including its directed monitoring failures. Recent outcomes retain bounded rendered results. Summary delivery and SSE recovery preserve this independent observation without treating a work update as repair of a stale transcript.

Private request pages use authorized detail reads and declared shared response bindings. Preparation failures are distinguished from uncertain execution. A separate private observation reports form validity; resolution, generation change or disconnection clears secret input and disables obsolete controls. No secret fields enter composer storage. Dismissing the page leaves domain work running.

The real scoped HTTP fixture exercises credential opening, stale-generation rejection without secret echo, cancellation by another view, private observation resolution, tool approval and duplicate-response rejection, and exact accepted prompt output in recent outcomes. All34 web tests and Chromium checks pass; session138 tests also pass with explicit terminal facts in credential/approval summaries. Generic mapped action forms, provider/model preparation, overview/lifecycle controls and initial browser preferences before observing content remain outstanding. This is not a full web completion claim.

## Incremental implementation commits

The user authorized commits as work proceeds. The reviewed plan, shared client foundation, protocol/query/publication foundation, session operation machinery, kernel ownership/teardown, daemon lifecycle/delegation and real-WASM contribution contracts are now committed as coherent subsystem changes. The coordinated cutover remains unfinished: normal-path legacy removal, final client migrations, both lockfiles/source closures, and packaged artifact verification are still required.
