# Scoped observations, commands, and presentation

Status: implementation and completion plan, reviewed 2026-09-16. This records the agreed design and the work required to finish it; exit gates are not claims about current behavior. No implementation milestone is complete merely because an earlier experiment introduced similarly named types. The user has authorized incremental commits; pushing and releasing remain separate actions.

This plan supersedes the connection, synchronization, shared-panel, and client-composition assumptions in [plan.md](plan.md) and [architecture.md](architecture.md). Their implementation history and existing verification remain useful. Update those documents to describe the final implementation at cutover, rather than marking proposed behavior as already built.

## Outcome

A client connects to any number of daemons. Each connection can carry observations and invocations addressed to any permitted scope. Selecting a session or opening a presentation is local application state.

Owners expose named queries and commands. Observations select coherent results and replicate them efficiently. Commands request validated changes and return correlated results. Presentation is an optional family of query results, described by ordinary catalogs and rendered through capabilities the client already implements.

The same machinery supports a transcript, configurable status, a plugin's session pet, provider authorization, a pending tool approval, a headless consumer, and an overview of sessions and delegated work. None adds a new wire lifecycle.

## Design decisions

| Concept     | Contract                                                                                                                                                         |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Owner       | Authority, state lifetime, incarnation, serialized state transitions and publication boundary. A session and the daemon directory are initially separate owners. |
| Scope       | An address resolving to an owner's exported interface. Addressing a scope does not attach the entire connection to it.                                           |
| Query       | Installed, named, pure projection with arguments, declared dependencies, result contract and explicit export policy.                                             |
| Observation | Consumer-owned interest in a bounded selection of queries, with a typed replica, synchronization position, cancellation lifetime and fault/recovery state.       |
| Command     | Installed operation with input/result contracts, authorization, preconditions and an implementation using the owner's event/effect machinery.                    |
| Invocation  | One command request: target scope, caller context, request identity and input values. Correlation does not by itself imply retry deduplication.                  |

Operations, input requests, summaries, relationships, presentation contributions and action bindings are application schemas built from these contracts. They do not get separate transports or generic manager frameworks.

### Public API shape and boundaries

The shared client exposes a small set of operations (illustrative signatures, not a second specification):

```text
connect(daemon) -> connection
read(scope, selection) -> coherent result
observe(scope, selection, checkpoint?) -> observation handle
invoke(scope, command, arguments) -> correlated outcome
close(observation handle)
```

The connection supplies authenticated caller context; input never supplies its own authority. A selection is a bounded map of named exported queries. Read is finite; observe owns ongoing interest. Both use the same query/result contracts. Closing an observation releases interest, not domain work. Blob transfer remains an independently scheduled transport facility referenced by authorized domain results.

The owner registers interest and captures its initial publication under one synchronization boundary. On subsequent publications it computes affected demanded queries, assembles a coherent update, and advances each observation only after the whole result is valid. Reference-count observation interest separately from reusable query caches. Share evaluations only when arguments and visibility are equivalent. A required-query fault leaves the entire selection stale; recovery must recompute or otherwise prove every selected member current before publishing it as current again.

Replica application yields an applied-change description after validation/application: changed member identities, document changes and live-stream changes. Frontends use this to update retained rendering without cloning every incoming transaction or rebuilding whole snapshots. Invalid partial application exposes no new state; recovery replaces or repairs the invalid replica.

Handshake, authentication, framing, liveness, flow control and blob transport remain base transport concerns. Everything else is either queryable owned state or a directed invocation outcome. Token streaming is a typed update to an observed document's live overlay; sessions are directory records and independently addressed owners; operation progress is queryable state. Directed command replies are not broadcast state and must retain their own correlation/lifetime.

### What this preserves, and what it costs

Preserve the existing immutable values, declared query graph, event/effect separation, stable document identities, incremental canonical projection, append buffers, retained renderers and kernel credential boundary. Old Misa's status implementation already separates registered domain queries, configurable selections and formatting; preserve that separation rather than copying individual widgets into each new client.

This model gives up connection-implied session selection, a single universal view and the convenience of global event replay. It requires explicit scope/instance lifetimes, bounded observation bookkeeping and coherent publication. Cross-owner observations do not offer atomic joins. Portable presentations can carry less richness than a custom native implementation; producers supply meaningful supported alternatives, and clients can decline optional content. No generic mechanism can make every interface equally expressive.

Avoid compensating with a distributed transaction system, arbitrary client queries, a universal UI language or downloaded renderer code. Add specialized encodings or capabilities only when a concrete feature and measurements justify them. Keep consumer cache eviction invisible to domain correctness.

### Transactional composition

An observation selects one named query or a finite named product of exported queries in one owner:

```text
observe(session, {
    conversation: query("conversation.presentation"),
    model:        query("models.selection"),
    pet:          query("pet.presentation.compact")
})
```

The server evaluates affected projections against one owner publication, emits one update transaction, and the replica applies the complete transaction before notifying consumers. Unchanged members retain identity; only changed members need payloads. Installed query dependencies provide derivation. Client selection is data, not remotely supplied code or arbitrary database paths.

One-query observations use the same mechanism. Independent observations may advance independently. Cross-owner joins, even between sessions on one daemon, combine independently versioned observations and cannot claim an atomic source snapshot.

Owner publication positions are distinct from persistent log positions. Effect acceptance, in-memory state commit, durable recording, and external effect completion must not be described as the same event.

### Replication and persistence

One observation lifecycle covers opening, initial state, atomic updates, temporary faults, recovery and terminal closure. Opening must establish an atomic snapshot/watch boundary. Result faults cannot masquerade as successful data maps. Required-member failure faults the coherent selection; an explicitly optional member may contain a typed failure at that publication. Last-good data can remain visible as stale but cannot be relabeled current.

Use value replacement, existing stable-ID tree operations and checked text appends initially. A composed result carries member updates atomically. Add keyed collection operations only where required to meet measured costs. Do not add arbitrary network codec execution or a universal DB patch protocol.

An observation identity binds owner incarnation, exact selection/arguments and result encoding contracts. Wire handles include a generation or are never reused during a connection. Changing selection opens a new observation. A replica advances its position only after successful complete application. Invalid updates invalidate that replica and require recovery; other observations remain valid.

Invocations also bind to the intended owner incarnation through their scope handle. Reusing a session label must never route an old queued command or input response to a newly created session. Resume revalidates current export authority before replaying retained results; revoked observations close or reset with explicit loss of authority.

Preserve transient append buffers and the absence of token-by-token DB patches. Final content publication and live-text retirement belong in the same observation transaction, including interruption, cancellation and failure. Late provider chunks must be rejected using attempt identity.

The conversation presentation result owns both canonical content and its live overlay. Its minimal valid observation is complete; clients must not know to select a hidden companion query to avoid completion races. Pure durable conversation queries can still be exported independently with an explicitly durable-only contract.

Persist canonical data without requiring a disk write for every token. Persisted checkpoints explicitly describe retained members/state. Reopening a checkpoint that omitted live data must reset live state before appends, while retaining valid canonical replay when possible. An initial unsupported checkpoint format gets a safe reset, not an invented cursor. Full in-memory reconnect may resume the complete replica. No universal vector-clock system is required.

History, active observations, argument cardinality, buffered bytes and pending invocations are bounded. Slow observations can coalesce to a coherent current snapshot. They must not prevent other scopes, command results or local editing from progressing. Chunk framing alone is not fairness for a large logical snapshot.

Canonical replay and transient delivery have independent retention budgets even when publication is coordinated. Token appends must not consume canonical-history slots or immediately evict durable replay. Recover omitted live state by resetting it while replaying still-retained canonical changes; test this after a long stream, not only after one append.

### Commands, operations and interaction

Commands are independent of presentation nodes. An action binding names a command, supplies contextual arguments and maps input values to parameters. Palette entries, slash syntax, forms, buttons and model tools can invoke the same underlying operation under their respective authority.

The owner revalidates supplied identifiers, current permissions and operation-specific preconditions. Moving away from node-based dispatch must retain existing authoritative checks: for example, saving an attachment resolves an authorized attachment reference rather than trusting an arbitrary client-supplied blob hash. Stateful offers/input requests carry their identity and generation when that context is necessary. Ordinary commands do not need blanket session-revision preconditions or capability tokens.

Results are completed, rejected, accepted with an operation reference, or indeterminate when execution may have happened but a trustworthy result is unavailable. A malformed post-execution result cannot be called a rejection. Acceptance has an explicitly documented durability meaning. Client deadlines or lost replies do not imply domain cancellation. Uncertain writes are not automatically replayed. Supporting idempotent retry requires a separate declared and tested deduplication policy. Directed results such as editor recovery and download offers cannot be recovered by replaying a general subscription.

Long-running operations are ordinary owned records. Input requests belong to those operations, with input contracts, permitted responders, generation, resolution, expiry and cancellation rules. Resolving a shared request is transactional; first-valid-response behavior applies where declared. Operation progress and bounded terminal-result retention are independent of one client's connection lifetime; reopening a view queries retained authoritative outcomes and can explicitly encounter expiry.

Drafts and presentation instances are local, including separate tabs/windows on the same connection. Hiding a presentation is not cancelling the underlying operation. Credential values never enter ordinary shared snapshots, echoed results or logs. General authorization progress and restricted challenge detail have separate exposure rules.

### Presentation and capability selection

Plugins project domain meaning into supported presentation primitives. Clients understand structured content, values, fields, actions and defined media capabilities; they do not need to understand every plugin's domain.

An ordinary presentation catalog declares contribution identity, title, scope, variants, query references, required versioned capabilities and advisory selection hints. The client selects a compatible variant before observing its content. Requirements are flat memberships, with structured parameters where real limits demand them. Namespaces do not imply support inheritance. Prefer coarse contracts initially; avoid a capability-expression language or global solver.

Rendering capability, caller authorization and local preference are distinct. Variants are installed, inspectable projections; the daemon does not branch on client brand. Do not put theme, viewport, hover or animation clocks into shared domain state or its query cache keys. Unknown requirements lead to another advertised variant or an explicit unsupported presentation.

Portable variants retain essential content and commands. Rich presentations can use defined media/interaction capabilities, but a new scene or animation language is not a prerequisite for this refactor. Producer-authored variants establish the extension seam; validate it using existing text/forms/image capabilities. Clients own placement, visibility, focus, gesture mapping and styling. Action and domain identities survive variant switching; draft transfer occurs only between compatible stable fields.

### Sessions, overview and delegated work

Each session publishes a cheap domain summary: identity/incarnation, title, lifecycle, activity, active operations, authorized pending-request summaries and relationship references. Working and needing input are orthogonal facts, not mutually exclusive status enum values. An approval exists because an unresolved application request exists, not because a dialog is displayed.

The daemon maintains an observable directory from lifecycle changes and owner summaries. Directory snapshots are coherent versions of the latest received summaries, not simultaneous reads of every session. Entries carry source versions and explicit freshness/availability. Summary maintenance must not instantiate transcripts or plugin panels. Overview visibility must not leak restricted challenge details or unauthorized relationships.

Define session, operation and delegation separately. A delegation references parent work and delegated work, with explicit blocking, cancellation and lifetime policies. Spawning does not imply that the parent is waiting or that the child dies with it. Support relationships to a child session or an operation in the same session; execution initially uses the existing runtime's supported form. Do not require every task to become a session.

Attention rollups retain underlying request/operation identities, deduplicate descendants and distinguish a blocking child from independent background work. Relationship traversal must tolerate missing or inaccessible targets and reject invalid local dependency cycles. Cross-daemon views are local joins with explicit freshness, never distributed transactions.

Implement and verify a minimal explicit delegation lifecycle as part of this plan rather than claiming a relationship schema implements agents. The working tree now contains same-daemon execution and parent continuation; their presence does not complete the client overview, cancellation, restart and packaging gates. Remote scheduling, arbitrary cross-daemon task migration and automatic provider-effect retries are outside this refactor.

## Code ownership

| Location                   | Final responsibility                                                                                                                                                                                                                                          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `misa-value`               | Immutable data and internal state patches. No exported arbitrary state-path API.                                                                                                                                                                              |
| `misa-reframe`             | Installed query graph, bounded memoization, handlers, transaction/effect rules. No connection ownership or rendering.                                                                                                                                         |
| `misa-proto`               | Scoped addresses, schemas/references, observation/invocation envelopes, typed presentation and update vocabulary.                                                                                                                                             |
| `misa-protocol`            | Shared protocol state machines, replica validation/application and owner observation contracts; no socket or filesystem policy.                                                                                                                               |
| `misa-transport`           | Authentication/admission integration, framing, cancellation-safe network ownership, connection drivers, independent blob transport.                                                                                                                           |
| `misa-session`             | Session policy, authoritative operations, summaries, incremental projections and coordinated publication. Split runtime/query/publication/operation owners out of `lib.rs` as their responsibilities become concrete.                                         |
| `misa-daemon`              | Composition, daemon identity and lifecycle registry, session creation/closure, summary aggregation and delegation orchestration. Extract testable runtime modules/library from the CLI.                                                                       |
| New `misa-client`          | Shared daemon relationships, typed observation handles, bounded correlated invocations and transfer coordination. Pure interaction helpers may live in a distinct module here. No renderer or server implementation dependency.                               |
| `misa-kit`                 | Optional reusable interaction algorithms that really are surface-independent. Move terminal-specific selection and application-specific parsing/preferences to their actual owners. Rename or remove the crate only after its remaining consumers justify it. |
| `misa-render` and surfaces | Shared value formatting/semantic policies where appropriate; terminal lines, DOM, native scenes and local render caches stay surface-appropriate.                                                                                                             |
| `misa-plugin`, `wit`       | Validated contributions to queries, commands, owned state/effects and presentations; explicit ABI/versioned contracts.                                                                                                                                        |
| Android JNI/Kotlin         | Shared client integration, storage and lifecycle adapters, platform rendering. Complete transaction callbacks across JNI.                                                                                                                                     |

Only one new core crate is proposed: `misa-client`. Operations, requests, summaries and variants do not each require a new crate. Keep the client normal dependency graph free of `misa-session`, `misa-kernel`, Wasmtime and Skia.

## Existing work: preserve, replace, verify

The working tree contains uncommitted experiments from before the model was settled. Record their ownership and validation before integrating them; do not reset unrelated work or call this tree a verified release baseline.

Current checkpoint, reviewed against the working tree: scoped observation/invocation contracts, coordinated session publication, typed replicas, multiplexed transport, an observable daemon directory, and a shared client exist. TUI, web and native desktop have scoped integration work. Android's Rust bridge and Kotlin state owner are being migrated, but its UI and instrumentation are incomplete. Query/command/presentation catalogs, real WIT contributions, prompt operations, private credential requests, approvals, operation checkpoints, delegation execution and overview projections have implementation and focused verification. See [refactor-progress.md](refactor-progress.md) for chronological evidence. This is not a completed application migration: workflow adapters, platform UX, lifecycle edge cases and final deletion still have outstanding gates. New edits are not verified merely because nearby earlier tests passed.

### Concrete integration obligations

- Capture all selected documents from one replica publication before handing a transaction to a renderer. Independently locking each member can mix publications even when the wire transaction was coherent. DOM, native and JNI delivery must preserve the same boundary.
- Reconfigure selected presentations asynchronously with bounded work. Keep the existing presentation until the replacement selection is ready; switch content and declarations together. A rejected configuration must not strand the composer or lose drafts.
- Resolve local presentation preferences before opening content observations. Conversation and status may be application defaults; optional plugin panels must not all be eagerly subscribed. Persist explicit hidden/automatic/variant choices with understandable invalid-preference recovery.
- Keep submitted text recoverable until its directed outcome is known. Preserve newer typing; expose rejected or uncertain submissions separately; never automatically replay an uncertain write. Retain every accepted operation reference, including overlapping prompts, rather than one global pending-turn slot.
- Bound parked presentation instances without abandoning already accepted domain work. Switching daemon/session preserves local drafts and pending outcome discovery. Browser duplication must create independent local instances even when tabs start from the same URL; expired instances need a draft recovery path.
- Finish generic restricted request forms in every interactive client, including secret fields, generation checks, validation, dismissal/reopening, expiry and resolution by another client. Provider-specific login is a producer of these requests, not a special client protocol.
- Bridge plugin tool declarations to installed commands explicitly. The current WIT descriptor's commands and presentations alone do not expose a model tool. The pet fixture must exercise both tool invocation and client action with the correct caller authority and one mutation implementation.
- Separate durable desired session membership, saved conversation logs and live runtime incarnations. Specify which sessions reopen after daemon restart; restore records without repeating unfinished external effects. Dynamic create/close must update the chosen membership policy durably before claiming restart persistence.
- Attribute background processes and delegated work to explicit lifetime owners before promising cancellation. A removed directory row or aborted observer is not proof that owned work stopped.

### Immediate remaining sequence

1. Consolidate the foundations already implemented. Audit transaction staging, publication, reconnect admission and resource teardown against stages 1–6. Exercise cancellation and command replies while restoring the maximum supported observation count; do not restart already completed architectural work.
2. Finish shared operation tracking and interaction adapters. TUI uses the shared tracker and schema-derived forms; migrate remaining frontend watchers. Preserve accepted operation identity even when monitoring fails. Cover expired results, closed owners, reconnect before completion, overlapping operations and cross-scope acceptance. Provider requests remain ordinary restricted operation data.
3. Finish each production surface. Web needs complete workflow discovery/forms, operation results, provider/model preparation, preference selection before content observation and expired-instance draft recovery. Android needs chooser/forms/rendering/instrumentation integration with the scoped Rust/JNI owner. Native needs final shared-tracker/form integration and artifact verification. Audit TUI/print parity against the original interaction regressions rather than equating unit tests with completion.
4. Give local preferences one persistence owner per application profile, with serialized updates; use locking or revision/merge semantics across processes. Atomic file replacement alone does not prevent two writers losing independent choices. Separate saved defaults from live instance selection and transient/secret drafts.
5. Finish actual lifecycle and overview scenarios. Verify shell/OAuth teardown completes before session close returns; parent continuation, independent child lifetime, cancellation propagation, restart interruption, request deduplication and direct/inclusive usage. Add multi-daemon overview navigation in every interactive surface without transcript polling.
6. Remove legacy owners and envelopes, separate pairing from the old session protocol, reconcile persistence and package closures, and run the complete artifact/scenario matrix. No preparatory adapter counts as frontend completion while its production path still uses Attach or ClientView.

This sequence prioritizes finishing the existing foundations; the numbered stages below retain the full design, ownership, and acceptance obligations.

| Work                                                                                              | Disposition                                                                                                                                               |
| ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Composer-owned picker filtering, resident candidates, Ctrl-D semantics, cancellation-safe input   | Preserve behavior and focused tests; route requests through the new client.                                                                               |
| Daemon stdin thread and Ctrl-C/interactive EOF fix                                                | Preserve and verify with real PTYs; noninteractive `/dev/null` must not terminate a service.                                                              |
| Local advertisement/pairing and persistent client identity                                        | Preserve intent; validate same-user trust, private paths, stale entries, socket path length, full identity checks, concurrent starts and cleanup.         |
| `transport::Daemons` and static session greeting                                                  | Replace static directory and separate attachment ownership with observed daemon scope and shared connection management.                                   |
| TUI `Workspace` and web `Hub`                                                                     | Salvage local selection UX; replace duplicated network/session lifetime management. Remove global cookie selection and unbounded per-selection retention. |
| Indicator/domain-query split                                                                      | Preserve domain facts and reusable formatting. Remove special protocol/ABI machinery whose only purpose is status widgets.                                |
| `Section::query` with its own cache                                                               | Replace with the owner-managed query/projection graph. Avoid root invalidation and duplicated computation per presentation.                               |
| Universal `Vec<Line>` component output                                                            | Remove from web/native component integration; preserve actions, identities, styling semantics and native interactions.                                    |
| Existing indexed trees, retained rendering, stream append code and snapshot/event watermark tests | Preserve as implementations and regression oracles, changing lifecycle around them.                                                                       |
| Current WIT indicator additions                                                                   | Fold into the planned query/command/presentation contribution contract before declaring an ABI stable.                                                    |

## Execution order and gates

All stages are reviewable work on one refactor branch. Preparation can land independently; the wire/ABI cutover is one coordinated integration across clients. Do not ship a mixture of old/new semantics, permanent compatibility shims, or two competing client owners. Make focused conventional commits as work proceeds, as requested. Stage explicit paths or hunks; preserve unrelated changes. Each implementation commit records its relevant verification and remaining limitations. Do not push or release without separate authorization.

### 0. Establish the migration baseline

Tasks:

- Inventory current diffs, test fixtures, local state/checkpoint formats, plugin ABI and all wire consumers, including Kotlin, browser JS and print mode.
- Re-run the baseline gates against the actual Cargo target directory and explicitly identified binaries. Real-process checks must not accidentally execute stale `next/target` artifacts from another invocation directory.
- Preserve user-facing input/shutdown fixes and record existing failures separately from refactor regressions.
- Replace obsolete assumptions in the design specification: connection equals attachment; one tree for every possible presentation; capabilities always forbidden; panel visibility shared by a session; subagents/session lifecycle out of scope.
- Capture workload evidence for small/large transcripts, Unicode token streams, several observers and overview summaries. Record work counts, bytes, allocations where available, and controlled elapsed measurements only when meaningful.

Exit gate: reproducible baseline, exact consumer inventory, and a test matrix with existing versus newly required guarantees distinguished.

### 1. Define contracts and executable protocol fixtures

Paths: `misa-proto`, `misa-protocol`, schema/fixture tests, architecture specification.

Tasks:

- Define scope identity/incarnation, query/selection contracts, observation handle/generation, typed snapshot/update, recoverable fault, terminal closure and invocation/result envelopes.
- Define modest reusable argument/result schemas and query references; separate semantic field data from presentation hints.
- Specify checkpoint behavior for canonical and transient state, cancellation lifetime, retry uncertainty, acceptance versus completion, and visibility rules.
- Define atomic update framing: chunk assembly cannot expose a partially applied transaction. Include logical message bounds/recovery and scheduling policy.
- Build fake owners and replicas exercising the contract independently of sockets or presentation.
- Reserve a wire protocol version and plugin ABI change for coordinated cutover; incompatible versions fail clearly before domain requests. Existing ticket strings may remain address/session hints without restoring the Attach lifecycle.

Exit gate: round-trip fixtures, malformed/stale/foreign-epoch rejection, atomic application, fault recovery, unsubscribe generation safety and no invocation replay all tested.

### 2. Make query composition and contribution validation reliable

Paths: `misa-reframe/{lib,scope}.rs`, `misa-session/{contribution,usage,indicators,views}.rs`, `misa-plugin/src/host.rs`, `wit/policy.wit` and guest fixture.

Tasks:

- Separate leaf reads from derived computations; derived computations receive declared dependencies and arguments, not an undeclared whole-DB escape route.
- Add real query failure propagation. Remove plugin fault-as-success maps.
- Validate duplicate names, query arguments, fixed dependencies, cycles, exported contracts and command/presentation references during composition. Validate dynamic dependencies during evaluation.
- Keep owner-level query caches distinct from reference-counted active observations. Preserve immutable sharing and optional previous-result hints without making cache retention necessary for correctness.
- Specify sequential handler composition: later handlers read the transaction's accumulated working state, while a failed dispatch chain rolls back together. Reconcile the plugin adapter's documentation and tests with that contract.
- Separate recomputation from output change. Reuse computation for equivalent authorized reads, with caller-dependent projections partitioned correctly.
- Implement bounded selections over installed exports. Internal queries remain private unless explicitly exported.
- Extend WIT query declarations with dependency/read contracts and schemas. Derived plugin queries must not serialize the entire DB every time. Preserve declared-root confinement and failure containment.
- Authorize plugin read-root grants at installation. Bind contributed presentation query references against the final owner registry after all contributions are installed, including cross-plugin dependency/cycle validation; do not construct an incomplete registry per plugin or a second presentation query engine. Migrate daemon registration, real guest fixtures and guest lockfile together.
- Implement domain aggregates using shared collection ownership or explicit incremental aggregates, with fresh-rebuild equivalence. Do not treat repeated full ledger scans as an automatic benefit of generic subscriptions.

Exit gate: coherent multi-query selection; unrelated writes preserve results; eviction changes performance only; duplicate/invalid compositions fail; one observer closing does not affect another; real WASM fixture can contribute a non-UI query and recover from a query fault.

### 3. Coordinate owner publication and typed replicas

Paths: `misa-session/{lib,canonical,protocol}.rs`, `misa-protocol`, `misa-proto/src/sync.rs`.

Tasks:

- Introduce a single owner publication sequence/boundary coordinating committed DB state, incremental projection advancement and transient stream changes.
- Reuse `Canonical`, stable tree IDs and append buffers behind the observation contract. Preserve rebuild as the correctness oracle, not the update algorithm.
- Publish settled message insertion and transient retirement together. Handle finish, cancellation, interruption, error and stale provider events.
- Evaluate/publish only demanded presentations; owner-maintained summaries and domain bookkeeping have explicit independent lifetimes.
- Make snapshot plus observation registration race-free. Recover from retained updates or coherent current state with a captured watermark.
- Centralize replica validation, application and cursor ownership. Return typed applied updates so renderers can invalidate only affected owners; do not expose only whole snapshots.
- Implement checkpoint restore that resets omitted transient state without replaying append offsets into nonexistent text.
- Begin the minimal `misa-client` integration here: carry one composed observation and one ordinary command through a real network connection into a TUI adapter and a headless consumer. Use the already specified invocation envelope; do not wait for full login, delegation or session restoration. Stage 6 completes this owner rather than introducing another client.

Exit gate: reproduce and eliminate End-before-canonical publication; no half-visible composed updates; snapshot/append races converge; partial invalid batches do not become visible; token work and wire bytes scale with appended content, not transcript history; the minimal real-client API has been exercised.

### 4. Unify command definitions, invocation and operation state

Paths: `misa-session/{agent,catalog,replies,protocol}.rs`, `misa-proto`, `misa-protocol`, `misa-kernel` request/effect boundaries, WIT host/guest.

Tasks:

- Register command input/result contracts and handlers. Map prompt, interrupt, cancel, model/effort changes and contributed actions through one validated invocation path.
- Specify guest command outcomes in the WIT contract. A null admission result is not a completed durable mutation. Either restrict and document a genuinely synchronous command or expose accepted operation/result semantics for journaled or asynchronous work; do not leave a second weaker plugin command lifecycle.
- Replace view-node authority with authoritative entity/offer/input-request checks. Keep bound arguments separate from user values; no client-controlled caller identity or recipient routing.
- Move completion to finite query execution using the same source definitions, preserving request correlation, limits and truncation.
- Generalize the existing reserved directed reply path for reliable results. Return editor handback and download offers only to their originating invocation.
- Introduce operation/input-request records for actual long work; define acceptance durability, terminal outcomes, retention, responder rules, expiry and cancellation.
- Convert provider login/authorization and shared approvals to these records. A read-only status report becomes a query; ordinary command preparation remains local.
- Preserve kernel ownership of credentials and external effects. Make failure and crash outcomes explicit without claiming externally exactly-once behavior.

Exit gate: palette, form, tool and headless calls reach the same operation with proper caller authority; rejected/stale/duplicate responses do not mutate state; disconnect does not replay commands; dismissing a dialog leaves the operation running; secrets are absent from replayable data; downloads and editor handback are directed.

### 5. Add daemon ownership and multiplex scopes through transport

Paths: daemon runtime modules, `misa-transport/{iroh,actor,server,admission,local,identity,daemons}.rs`, `misa-protocol` routing.

Tasks:

- Extract daemon lifecycle/state from `main.rs`. Add create/list/close session commands with incarnation, resource teardown and terminal observation closure.
- Closing a session stops its runtime according to explicit cancellation policy; it does not delete the conversation log. Reopening/resuming creates a new runtime incarnation. Session labels are not sufficient global identity.
- Reject invocations targeting a previous incarnation, including delayed input responses after scope recreation or restart. Revalidate export authority on resume and invalidate affected queued snapshots/history on permission changes.
- Replace the fake empty `SessionInfo` greeting with handshake identity/version information and normal directory/declaration observations.
- Remove connection-wide Attach and session fields. Route observations/invocations to owner backends by scope.
- Maintain one authenticated connection owner per daemon and a reusable local client endpoint/identity. Deduplicate concurrent connects; retain admission checks and independent blob transfer.
- Add durable daemon identity in configured state storage, separate from process/scope incarnation. Define restart restoration of saved sessions without reissuing unfinished provider/tool effects.
- Complete local discovery/pairing using supported platform mechanisms, bounded stale-entry handling, full identity verification and explicit connection failures. Pairing one local client must not consume an unrelated remote invitation.
- Bound queues and pending work; keep ordinary scheduling simple, with fair progress across ready observations and control results. Revoke/close affects only the appropriate relationships.

Exit gate: multiple scopes on one daemon connection; two daemons with identical session labels; directory changes while viewing another session; deleting/recreating one scope leaves others healthy; daemon restart is distinguishable from client reconnect; slow snapshots do not starve commands.

### 6. Build the shared client and prove an early vertical slice

Paths: new `misa-client`, workspace manifests, transport/protocol handles, TUI adapter first, minimal adapters/tests for the other consumers.

Tasks:

- Expose connect/disconnect, finite read, observe/cancel, invoke and blob transfer handles. Allocate request/subscription IDs centrally rather than hashing source names into small ranges.
- Own typed replicas and pending results once. Provide generation-safe lifetimes, bounded pending requests, timeout/uncertain-result semantics and observable connection freshness.
- Keep transfer fetching/decoding off the protocol receive loop. Retain the TUI request coordinator's useful bounds and cancellation behavior.
- Add small pure interaction helpers for command preparation, source resolution and pending-request responses. Keep selected scopes, open presentations and drafts outside the remote core.
- Drive one real TUI transcript plus a non-UI data query and daemon directory through the new path early. Use a headless client test so rendering does not hide domain API gaps.
- Test a non-UI consumer and an incremental renderer against the same client API before completing the rest of the migration.

Exit gate: a single client simultaneously observes two daemon directories, two sessions' usage and one transcript; background updates survive foreground selection changes; local editing stays responsive during reconnect, completion and file transfer; handles release resources correctly.

### 7. Implement presentation catalogs and local presentation instances

Paths: session/plugin contribution definitions, client interaction modules, `misa-render`, `misa-kit`, frontend component adapters.

Tasks:

- Declare portable presentation variants and capability requirements as ordinary catalog data. Validate variant query/result references and intentional portable alternatives at installation.
- Implement client filtering and local preference selection before content subscription. Unsupported capabilities never cause executing remote renderer code.
- Support independent instances with separate field drafts, expansion and visibility. Change variants by opening a fresh observation; preserve compatible stable input identities only.
- Split command schemas, presentation fields and value submissions. Bind actions declaratively to commands and keep gestures local.
- Replace the session-wide panel singleton with query-backed local reports and operation/request presentations. Preserve discoverability of actionable pending requests even when a presentation is dismissed.
- Rebuild status as configurable composition over domain queries and local client facts, with registered value formatting and surface-appropriate layout. Port provider/model/effort/context/quota semantics from old Misa, including zero, unavailable and missing-limit distinctions.
- Make provider/model catalogs available without requiring a first prompt. Refresh provider-owned facts on explicit domain lifecycle events, not UI redraw. Never reuse one provider's quota as another's.
- Keep themes and presentation preferences local. Remove terminal-line component output from web/native adapters; preserve actions and native accessibility.
- Add a real plugin fixture with session pet state, commands, summary and a portable text/forms variant plus an existing-capability richer variant. Tool calls and client actions mutate the same state. One client hides it; another observes it.

Exit gate: unknown plugin domain renders through known portable primitives; only selected variants compute; local panel changes do not affect another client; capability and authorization checks remain separate; two pet representations invoke the same commands; status is configurable without renderer branches per indicator.

### 8. Session summaries, delegation and overview

Paths: session summary/operation modules, daemon directory/delegation modules, kernel attempt metadata, client overview projections, tool/command declarations.

Tasks:

- Publish cheap session/operation summaries, including source incarnation/version, independent activity/attention facts and permitted request references.
- Maintain directory summaries from lifecycle/publication notifications. Avoid polling transcripts or copying full session state into the daemon catalog.
- Define stable delegation records with parent operation, target operation/session, blocking relationship, cancellation propagation and independent lifetime.
- Keep session, conversation, operation, delegation and provider-attempt identities distinct. Propagate attempt relationships through stored/exported records; do not infer delegation from one reused ID or a parent label.
- Implement minimal delegated work creation, execution, result delivery and parent continuation through declared commands/effects. The command explicitly supplies task/context and configuration overrides; credentials remain daemon capabilities and authority cannot be escalated by inheritance.
- Use a deterministic same-daemon execution path first. Record relationships and results durably where restart survival is promised; interrupted non-idempotent work is surfaced, not automatically rerun.
- Stage durable terminal decisions until checkpoint acknowledgement, then publish them. The current delegation implementation can publish success before persistence and retract it on failure; fix this before claiming terminal durability. Inject crashes/failures before and after each acknowledgement boundary.
- Define bounded terminal retention with eviction or archival and explicit expiry/forgetting, rather than a lifetime record cap that eventually prohibits new work. Bound attempt metadata separately from result bytes. Return stable conversation and logical-operation reconciliation references when ephemeral child scopes close, including oversized or uncertain results.
- Await lifecycle admission, owner teardown and supervisors during daemon shutdown before dropping the executor. Explicit session-close correctness alone does not establish process-shutdown correctness.
- Support explicit cancellation policies and terminal outcomes. Ensure a child approval can block its own work and be rolled up without inventing a parent-wide blocked state when other work continues.
- Derive deduplicated attention and dependency summaries. Handle cycles, removed targets, unavailable daemons and restricted requests without leaking content or counting one request repeatedly.
- Attribute usage to unique attempts, distinguishing direct totals from descendant-inclusive totals. Parent/child overview rows must not double-count the same cost or tokens.
- Compose a multi-daemon overview locally from observed directories. Show stale/unavailable facts honestly and navigate to authoritative detail/request observations for actions.

Exit gate: parent with working child, approval-blocked child and independent background child; one response resolves the authoritative request across observers; cancellation follows the declared policy; restart exposes interrupted work; overview never opens every transcript and never reports a disconnected session as idle.

### 9. Complete every surface and persistence adapter

Paths: TUI/print, web Rust/JS, Skia connection/window/app, Android native and Kotlin, kit/prefs migrations.

Tasks:

- TUI: preserve composer-owned filtering, resident/on-demand sources, paste, Ctrl-C/Ctrl-D, scroll/selection, retained lines and local theme/status configuration. Scope drafts/preferences by daemon identity and session/presentation instance.
- Print mode: use the same client contract and directed results without opening UI; define explicit selection behavior when several sessions are available.
- Web: replace unbounded intent/pending-selection lifetime with shared handles; make presentation/session selection tab-instance-local. Apply a complete observation transaction before DOM consumers see it. Preserve subtree updates, form drafts, browser selection and SSE recovery.
- Skia: replace manual pair/attach/receive loop; keep indexed layout, hit targets and retained paint groups. Fetch/decode assets asynchronously without blocking observation application.
- Android: migrate the independent native Cargo workspace, JNI contracts, Kotlin wire/tree/live-state application, connection/view-model ownership and saved checkpoints. Deliver complete transaction callbacks; background/foreground must not resurrect old handles.
- Use one authoritative replica path per observation. Surface indices may be derived render caches but must not independently own conflicting synchronization cursors.
- Use shared operation tracking and schema-derived form preparation across surfaces. Renderers own field controls and local editing, not competing operation state machines. Monitoring failure after acceptance must not turn the original invocation into a rejection.
- Serialize preference persistence through a local owner and test concurrent independent choices. Recover unknown saved variants with a per-presentation diagnostic while keeping other content usable.
- Remove old per-surface recovery/request code, fixed subscription-ID conventions, static directory wrappers and obsolete shared panel behavior.

Exit gate: the scenario matrix below passes on all shipped surfaces; platform-specific interaction remains native; dependency checks show clients link no server/kernel/plugin runtime. Android's separate tests are mandatory.

### 10. Cutover, packaging, documentation and deletion gate

Paths: protocol version, plugin ABI, both Cargo lockfiles, guest fixture, Nix source closures/packages, Android build integration, README/architecture/parity/verification docs.

Tasks:

- Update wire and WIT contracts, all consumers and fixtures together. Incompatible clients/plugins get explicit refusal. Document local checkpoint and preference migration; retain user drafts where possible and reset invalid replicas safely.
- Add `misa-client` to Android's filtered `nativeCrates` closure in `nix/android.nix`, its independent lockfile, workspace dependencies and required package source sets.
- Build packaged daemon/TUI/web/Skia/guest/Android artifacts with their existing pinned inputs. Update source/cargo hashes only from actual builds.
- Delete old Attach routing, fake session-directory greeting, view-only resume branches, unscoped stream delivery, completion-as-mutation, special download/editor-result wire paths and duplicate frontend coordinators once their replacements pass.
- Delete the obsolete WIT indicator-specific path and universal terminal component facade. Recheck public APIs for optional state-manager/framework layers that have no concrete second consumer.
- Update architecture and parity to describe final behavior, including transactional selections, scope-local consistency, presentation variants, operation visibility, session lifecycle and actual delegation support. Replace obsolete planning statements rather than maintaining contradictory active plans.
- Record exact artifact verification and unresolved platform limitations. An unavailable emulator/display is an outstanding gate, not a pass.

Exit gate: no old lifecycle remains in normal code paths; required tests and packaged checks pass; documentation reflects demonstrated behavior. Release/tag/push are separate actions and not authorized by this plan.

## Dependency and review strategy

Stages 1–3 establish contracts and publication, with a narrow real-network/client slice at the end of stage 3. Stage 4 completes the command/operation side. Stages 5–6 complete multiplexed ownership and the shared client. Stages 7–8 add application compositions. Stage 9 starts its surface work alongside the early client slice and completes after stages 7–8; it is not postponed until the server is finished. Stage 10 is the coordinated public cutover.

Review boundaries follow these responsibilities, with tests and old-path deletion attached to the relevant change. Keep the refactor branch coherent and each commit reviewable; do not claim each preparatory commit is independently releasable. No estimated duration substitutes for an exit gate.

## Acceptance matrix

| Area                 | Required evidence                                                                                                                                                                                                                                                                                                |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Coherence            | A transaction changing several selected queries is observed atomically; unselected changes are silent; required query failure cannot create a falsely current mixed result.                                                                                                                                      |
| Original regressions | No-argument startup discovers and auto-pairs local daemons without consuming a remote invitation; ambiguity opens local selection; picker typing/paste edits the composer; daemon Ctrl-C and interactive empty Ctrl-D exit; noninteractive EOF leaves the daemon running. Real-process/PTY evidence is required. |
| Streaming            | Unicode byte offsets; stale attempt chunks; snapshot racing append; finish/cancel/error with durable content and transient removal applied together; no token DB patches.                                                                                                                                        |
| Recovery             | Live reconnect, client process restart, owner restart, history expiry, invalid update, partial frame, scope deletion/recreation, checkpoint with omitted live state after many appends, independent canonical retention, revocation during snapshot and resume.                                                  |
| Multiplexing         | Two daemons with the same session label; simultaneous scopes; slow observation does not starve another scope or command result; unsubscribe with queued data; selection replacement.                                                                                                                             |
| Commands             | Same operation from several bindings; schema/authority validation; resource-specific stale preconditions; old-incarnation calls rejected; accepted versus completed; uncertain write; no blind replay; directed recovery/download.                                                                               |
| Interaction          | Two clients and two browser tabs with independent chooser/dialog drafts; local dismiss versus domain cancellation; shared approval resolution; restricted login detail.                                                                                                                                          |
| Plugins              | Real guest dependency declarations and faults, declared-root confinement, command/result schema validation, lazy variants, pet state/actions presented without domain-specific client code.                                                                                                                      |
| Overview/delegation  | Working plus attention simultaneously; cheap directory updates; child dependency/cancellation/lifetime; deduplicated requests; missing/cyclic/inaccessible relationships; honest freshness and restart behavior.                                                                                                 |
| Presentation         | Configurable status and theme; model/provider enumeration at startup; compact/richer variant selection before query; no lost fields/actions; failed variant can be replaced safely.                                                                                                                              |
| Cost                 | Existing tree/rebuild equivalence, history-independent append/update byte counts, retained renderer work, bounded queues/history and observer cleanup. Generic composition must not silently introduce whole-result copies per token.                                                                            |
| Packaging            | Desktop workspace, real WASM guest, browser DOM, native window/clipboard, Android native tests and packaged APK/emulator, dependency closure checks.                                                                                                                                                             |

Run targeted checks at each change, then the existing broader gates at integration. Relevant entry points, to execute in the pinned environment where required:

```sh
cargo test --manifest-path next/Cargo.toml --workspace
cargo test --manifest-path next/Cargo.toml -p misa-plugin --features guest-fixture
cargo test --manifest-path next/android/native/Cargo.toml
nix-build next -A packages.checks
nix-build next -A packages.misa-daemon
nix-build next -A packages.misa
nix-build next -A packages.misa-web
nix-build next -A packages.misa-skia
nix-build next -A packages.misa-guest
nix-build next -A packages.misa-android
nix-build next -A packages.misa-android.installCheck -o result-refactor-android-check
result-refactor-android-check/bin/misa-android-check
```

`misa-tui/tests`, `misa-web/tests`, `misa-skia/tests`, Android instrumentation and existing retained-work tests remain behavioral evidence; adapt their contracts rather than replacing them with implementation-mirroring assertions. Provider tests use controlled fixtures. Do not log into real external accounts just to validate workflow structure.

## Completion criteria

The refactor is complete when:

- A new exported domain query needs no presentation or transport changes.
- A new presentation contribution uses declared queries/capabilities/actions and requires no plugin-domain interpretation in every client.
- A new command uses the common schema, validation, invocation and result machinery.
- A client can independently observe several scopes on several daemons, with coherent compositions and bounded recovery.
- Shared workflows and per-instance interaction have distinct lifetimes, visibility and cancellation semantics.
- Session and delegated-work overviews derive from cheap authoritative summaries, not UI state or transcript polling.
- Existing incremental rendering, streaming, durability boundaries and platform interaction guarantees remain demonstrated.
- The superseded implementations are gone and all shipped artifacts use the same model.
