# Misa architecture

This describes the scoped client/daemon implementation. Verification results and
unfinished artifact gates belong in [refactor-plan.md](refactor-plan.md); this document does not
assert that a previously tested artifact contains today's source.

## Ownership and dependencies

A daemon hosts owners. A session is one owner, with a database, agent policy,
operations and exported queries. The daemon directory is another owner. A client
can connect to several daemons, and several clients can observe the same session.
Connecting, choosing an owner, choosing observations and choosing a local layout
are separate lifetimes.

| Component                         | Responsibility                                                                                                             |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `misa-value`, `misa-reframe`      | Immutable values, declared roots and query dependencies, transactional event handlers, patches and effect descriptions     |
| `misa-kernel`                     | Provider/tool capabilities, credential storage, conversation journal, blobs and attempt ledger                             |
| `misa-session`                    | Agent transitions, operation/input-request lifecycles, domain queries and semantic document projections                    |
| `misa-daemon`                     | Admission composition, active owner registry, durable desired session membership, archive discovery and delegated work     |
| `misa-proto`                      | Scope, schema, query, observation, invocation, semantic document and framing contracts; no policy or IO                    |
| `misa-protocol`                   | Validated replica, bounded invocation dispatch and authorized owner routing over injected traits                           |
| `misa-transport`                  | iroh connections, bounded framing, pairing, independent output lanes and blob transfer                                     |
| `misa-client`                     | Shared daemon relationships, observation leases, reconnect, finite reads, invocation results and local form/catalog models |
| `misa-kit`                        | Local editor, picker and command parsing; no session/kernel dependency                                                     |
| `misa-render` and surface clients | Semantic formatting, theme, layout, retained rendering and local interaction/storage                                       |
| `misa-plugin`                     | Bounded Wasm implementations of the composition's handlers, queries and declarations                                       |

Terminal, web, Skia and Android production clients consume shared contracts and
client machinery rather than a concrete session runtime. Android links the
shared Rust client through JNI. The web server is itself a scoped client and
translates its selected documents into HTML/SSE.

## Small shared contracts

### Scope and authority

A daemon relationship together with a `Scope` identifies the logical owner and its incarnation. A reused
session label is not the same running owner. Every selection and invocation
names its intended scope; an old queued call cannot address a replacement owner.

Transport authenticates the peer and constructs a trusted `CallContext` with a
principal and connection identity. An injected resolver authorizes access to an
owner. Owner lookup and execution receive this context; invocation input is not
an identity claim. Admission is checked during the relationship, including
observation polling. Revocation wakes and closes access.

### Query and selection

An installed query declares its argument schemas and result contract. A result
is data or a semantic document. A `Selection` is a client-chosen, named product of
members from one scope. Its snapshot/publication is coherent within that owner.
There is no implicit transcript, status bar, hidden subscription or universal
whole-database export.

A finite `Read` returns one correlated snapshot and retains no observation.
`Observe` retains a selection until cancellation or closure. Several client
features can share a lease through the client core. Changing local placement or
opening a report does not mutate the session.

Public pure queries use the declared dependency graph and memoization.
Restricted exports, such as a private input challenge, use an installed
context-aware evaluator outside the shared graph. They cannot be reached by a
guest forging a principal argument or naming a private derived dependency.

### Publication and recovery

An observation starts with a complete snapshot. Subsequent publications contain
named member replacements or document changes. A document owns a canonical tree,
a version and its live streams. A publication applies all included values, tree
operations and stream updates together; observers never see settlement halfway
through durable insertion and live-text retirement.

Canonical changes retain stable-ID insert/remove/replace operations. Live text
uses current values and UTF-8 byte-offset appends. Canonical history and live
publication retention have separate budgets: token traffic does not consume
canonical revision slots.

The replica checks scope/selection, handle generation, publication continuity,
document versions, tree operations and stream offsets. Invalid application
exposes no partially advanced state. Faulted or incomplete replicas require
complete recovery before sparse updates resume. Applied notifications retain the
owned incoming operations, so renderers can process an insert followed by a
remove in the same transaction without reconstructing it from the final tree.
They do not clone the entire existing transcript per token.

A full in-memory reconnect can resume the retained live replica. A canonical-only
disk checkpoint explicitly omits live state; recovery resets live streams while
replaying retained canonical changes where possible. Owner incarnation changes
and unavailable history require replacement. Client handles have generations,
separate from connection callback generations, so superseded recovery traffic is
ignored. Invocations are never automatically replayed on reconnect.

### Invocation, operations and requests

A command declares input/result schemas. An invocation names a command in a
scope and carries a correlated request ID. The owner validates input and
revalidates authority and domain preconditions at execution. Version expectations
belong to the particular command's input, not an invented universal transaction
precondition.

Results distinguish `Completed`, pre-execution `Rejected`, `Accepted` with an
operation reference, and `Indeterminate` when effects may already have happened.
A bad post-execution result must not be described as a safe rejection or retried
automatically. Dropping an invocation wait releases caller bookkeeping; cancelling
owned work is a separate explicit command.

Long-running prompt, credential, delegated and contributed-command work has
queryable operation records. `Accepted` is a receipt, not a promise that the work
or receipt is durable. Gated effects wait for their authoritative journal
acknowledgment. Restart reconciles interrupted operations without replaying their
external effects. Terminal input outcomes become visible coherently only after
their checkpoint and successful owner publication.

Input requests carry generation, responder authority, deadline, state and typed
response contracts. First valid resolution wins. Credentials use restricted
kernel-owned sinks: credential values do not enter shared DB patches, guest
events, snapshots or error echoes. Generic declared forms support nonsecret typed
records and an owner-validated continuation. Cancelling authorization differs
from dismissing a local form; cancellation races settle from authoritative kernel
outcomes.

Action bindings name an installed command and declaratively bind authoritative
inputs. Client-entered values cannot override those bindings. Commands marked
`Request` are prepared only through a private request model; generic command and
action forms reject them. This is preparation policy, not a substitute for owner
authorization. Local gestures/actions are not server command targets.

## Presentation and interaction

Domain data, presentation data and surface rendering remain distinct. A semantic
node describes roles, typed facts, structure, stable identities and declared
actions. It does not select fonts, colors, coordinates or keyboard behavior.

Presentations are optional exported capabilities with a finite set of declared
variants. Clients choose which members to observe and where to place them. A
plugin can contribute portable/rich alternatives without being forced into a
canonical transcript or making every client evaluate every variant. A composed
selection gives coherence for the chosen members, not simultaneous evaluation of
all possible UI choices.

Status is an ordinary query/presentation composition. Reports such as `/usage`
are finite reads; `usage.report` exposes data and `usage.presentation` supplies
semantic money/token/quota facts. A client owns the report window and dismissal.
A private request is owned domain state; the dialog's draft, focus, placement and
visibility are client state. There is no shared singleton panel controlling all
clients.

The client owns editor/picker filtering, selection, scroll, disclosure, theme,
unsent drafts and keyboard hints. Resident completion sources are observed and
filtered locally; on-demand completion is a bounded finite query. Catalog members
name their exact queries; clients do not synthesize IDs from source names.
Preferences are scoped by daemon and local session/presentation instance.

## IO, admission and backpressure

`/misa/scoped/3` greets an authenticated daemon and carries finite reads,
observations, cancellation and invocations. It has no attach message or session
catalog greeting. `/misa/pair/1` is a separate bounded admission-only protocol.
`/misa/blob/0` moves content-addressed bytes with its own size/hash validation.
A destination path for a received file stays local to the requesting client.

Each observation has an ordered server-initiated stream. Finite reads have
separate response lanes; control and invocation results use the control stream.
A blocked large document therefore need not queue ahead of another observer or
a command reply. Active lanes, logical frame sizes, channels and buffered bytes
are bounded. Readers retain partial framing across cancelled waits; writers own
whole writes. Observation writers poll coalesced fresh state when ready rather
than enqueueing every intermediate snapshot. The current scoped logical-message
limit is 64 MiB; this is an explicit transport bound, not a schema cardinality
limit or a promise of unlimited snapshots.

The browser additionally bounds SSE retention and repairs lagged viewers from a
complete selected presentation. Skia retains layout and paint groups; whole-window
raster repaint remains a cost. Terminal retained rows and Android's indexed
canonical state likewise derive from the shared replica rather than owning a
second protocol cursor.

## Database, kernel and plugins

The loop retains immutable structural sharing, declared roots/dependencies,
ordered handlers and bounded dispatch chains. Handlers consume their supplied
transaction state; effects are descriptions checked by the interpreter and run
after the relevant commit. Faults roll back a transaction. Explicit set/merge/
delete/append operations replace magic patch sentinels; handlers cannot replace
the whole database. Network publication positions, canonical document versions
and journal acknowledgments have different jobs and are not interchangeable.

The owner reserves bounded capability capacity against the finalized transaction
before publication. Refused admission rolls back the entire transition. Reservations
follow deferred checkpoints and queued/running requests until their last guard is
released. External work and completion/control work have separate budgets; the
serial journal lane can progress while file/provider workers are saturated.
Shutdown fences admission, interrupts owned work and awaits capability cleanup.

Transport retains admitted command execution independently of a connection's reply
waiter. Disconnect cannot cancel or replay that execution; protocol shutdown drains
it. This lifetime is distinct from a long-running operation's durable domain record.

The kernel owns provider adapters, credential injection/refresh, tools, durable
conversation facts and uniquely identified attempt accounting. Policy decides
turns, retries, compaction and continuation. Provider usage is a named capability
with normalized facts; policy does not receive a general HTTP credential escape.
The reserved kernel ALPN is not an implemented remote kernel: shipped sessions
use the in-process `Kernel` trait.

Wasm plugins declare roots, handlers, query dependencies, exported presentations,
commands, bindings, model tools and input-request forms in `wit/policy.wit`.
Composition validates these declarations. Guest calls are fuel-bounded and writes
are confined to declared roots. Guest event commands describe state transactions,
not arbitrary external-effect success; their operation settles on the journaled
transaction outcome. Model tools call the same installed command registry and
stay pending when it returns an accepted operation.

Contributed handlers share one transaction's working state. Their patches become
one staged journal decision; competing mutations wait or receive an explicit
busy result rather than silently overwriting unacknowledged state. Exact
acknowledgment publishes the staged patches and operation outcome. Automatic
reports use a bounded deferred queue; overflow is an explicit owner fault.
Private operation checkpoints are removed before restored log events reach guest
subscriptions. Restoring state never reruns effect descriptions.

## Daemon lifecycle and overview

The directory owns durable desired membership separately from active runtime
incarnations. Create/resume use an asynchronous factory and enforce a unique
conversation writer. Close retires the owner, closes observations and awaits
shutdown; removing a directory row alone is not sufficient. Archived conversation
discovery remains available when there is no active session.

Overview rows consume cheap session summaries, retaining source publication and
availability. They expose activity, attention and operation/request references.
The directory is coherent about its received rows, but those rows are eventually
consistent across different owners. It does not open every transcript or invent
a global atomic transaction.

Delegated work creates and executes real child sessions, correlates terminal
results and continues the parent tool call. Records declare parent-bound or
independent lifetime, deduplication identity and cancellation. Depth, retained
records/results and persistence are bounded. Restart retains relationships and
reconciles interrupted work without replay. Usage attribution deduplicates unique
attempts; inclusive totals describe retained relationships, not an eternal graph
after work metadata is deliberately forgotten.

`daemon.connections` reports authenticated daemon connections. Session observation
interest is a different fact; opening a document does not attach a client or make
it own that session.

## Evidence and limits

Meaningful gates include fake-owner protocol tests, real iroh pairing/reconnect/
large-document/fairness tests, replica rollback/checkpoint tests, canonical rebuild
and retained-renderer equivalence, durable operation failure/race/restart tests,
and actual delegated execution. The Wasm guest suite requires the explicit
`guest-fixture` feature and is not run by a default workspace test invocation.

Browser interaction, native display/clipboard, Android emulator installation and
packaged artifacts are separate gates. Their exact source/artifact evidence is
recorded in [plan.md](plan.md) and the relevant verification documents. Conversation
fork UX, moving an active owner between daemons, all-time relationship retention
and monetary execution budgets are not implied by the scoped protocol.
