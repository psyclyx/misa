# misa: layering, the boundary, and what is deliberately not here

This is the architecture the rewrite follows, stated so that the code can be
checked against it. It is a position, not a description: every claim below names
where it is enforced or which test pins it.

It replaces the previous system's four-layer map (`Interpreter`, `Kernel`,
`Policies`, `Presentation`) with five, because splitting presentation from policy
is not enough when five frontends must agree — the _shape_ of what is drawn has
to live somewhere, and it is neither a fact nor a drawing.

---

## 1. The five layers, and the direction of dependency

| Layer              | Crates                                                                        | Owns                                                                                              | May not                                            |
| ------------------ | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| **Wire**           | `misa-proto`                                                                  | the vocabulary: view nodes, intents, subscriptions, framing                                       | decide anything; perform IO                        |
| **Value and loop** | `misa-value`, `misa-reframe`                                                  | immutable state, patches, the event/effect/subscription machinery                                 | know what an agent is                              |
| **Kernel**         | `misa-kernel`                                                                 | durable facts, the attempt ledger, capability (provider, tool, log)                               | know what a turn is; know what anything looks like |
| **Session**        | `misa-session`                                                                | the agent loop, tools, the view tree, dialog composition                                          | draw; decide colour, size, or layout               |
| **Client**         | `misa-render`, `misa-client`, `misa-tui`, `misa-web`, `misa-skia`, `misa-cli` | theme, layout, wrapping, focus, interaction state, and the client-side machinery a surface reuses | own agent policy; invent agent state               |

Dependencies point one way: wire ← value ← loop ← kernel ← session ← client. A
crate never imports the one above it.

Three seams sit beside that line, and each exists because the alternative would drag a
layer somewhere it does not belong:

- **The protocol** is _both ends_ of one state machine: a session's end, which answers a
  connection, and a client's end, which attaches, subscribes and applies what comes back.
  Neither end has policy. The session's end reaches the session through a trait rather
  than a concrete `Runtime`, so a client build links no session and no kernel — not for
  tidiness: the phone links this crate, and it links `misa-session` and `misa-kernel`
  today to learn one query name.
- **The transport** is bytes: QUIC, the blob connection, the accept loop, and the tasks
  that pump one into the other. It is the only thing here that spawns.
- **The kit** is what the frontends share and the protocol does not own: the picker, the
  editor, the selection, and a client's own memory. It carries no IO, because a browser
  and a phone have their own storage and neither of them has a `$HOME`.

`misa-net` holds the protocol's two ends and the transport in one crate today, and `misa-client` is
the kit under a name that should mean the protocol's client end; `plan.md` phase 4 is the slice.

`misa-plugin` is the fourth seam: it implements the loop's own `Handler` and
`Subscription` traits over a wasm component, and depends on the loop, the wire, and the value
types — nothing above them. It is where a plugin stops being a boundary and becomes something
the loop runs, and it is a crate of its own because the runtime it needs (`wasmtime`) is the
largest dependency in this workspace and has nothing to do with a session.

One frontend is not a crate here: `android/` is a Kotlin app over the shared Rust
client, linked through a JNI seam, and it is a client for the same reason the others
are — it draws a tree it did not build and decides nothing about the agent.

The three layers the rewrite was asked for map onto this as:

- **presentation, owning no policy** — the client crates;
- **agent core, owning no presentation** — kernel;
- **the middle, where the agent loop and most plugins belong** — session.

---

## 2. The two invariants

Everything else in this document follows from these.

> **A session may not decide appearance.**

Enforced by construction: `misa_proto::view` has no field that can express a
colour, a font, a size, a weight, a coordinate, a row, a column, a duration, or a
transition. Adding one would be a visible change to the architecture rather than a
field on a struct.

The corollary matters more than the rule. Because a session cannot decide
appearance, it must carry _the facts an appearance decision needs_. A session does
not say "this thinking block is collapsed". It says the block is streaming, how
large it is, and that the node has a short form and a long form
(`Kind::Collapsible`). Whether the long form is showing is the client's, and two
clients with different tastes are both right.

The same rule reaches what a session knows about a client. One tree serves every client,
a client declares a name and a version and nothing a session acts on, and the only
per-client inputs to a view are the requests it makes. A session that branched on what a
client can draw would be deciding appearance by proxy.

> **A client may not invent agent state.**

Enforced by the message types. A client can send exactly five things —
`Intent::Prompt`, `Intent::Action`, `Intent::Command`, `Intent::Cancel`, and
`Intent::Complete`, which asks for candidates from a source the session declared —
and no message in the protocol can express an effect, a provider, a model
parameter, or a state path. A client may always change what it _shows_; it can
only _ask_ within that vocabulary.

`Session::intent` (`crates/misa-session/src/lib.rs`) is the only place an intent is
interpreted, and `view::validate` rejects a view node that offers an action using
the reserved `client.` prefix, so a session cannot name an affordance only a
client could honour either.

**What the client does own, and why "the client owns no agent policy" was not quite
right.** A client owns _interaction_ policy: which gesture means what, which
decision is local, and which is sent. That is presentation policy, and it is real
policy. What it must not own is policy _about the agent_ — retry, cost accounting,
compaction, tool continuation, history. That is the middle layer, and the previous
system's own migration plan (Slice 6, "split the agent loop") said the same thing.
The distinction that survives is not "client versus server" but **local decision
versus round trip**:

- local, always allowed: scroll, select, copy, expand, collapse, theme, focus, wrap;
- round trip: submit, resolve an action the session offered, run a declared
  command, cancel.

`misa-tui`'s `KeyOut` is that line made mechanical: `Local` is a local decision,
`Intent` is a round trip, and `Copy` is neither — it is what only the client can do,
because only the client knows what was selected.

A client's memory of itself is a file, and it is presentation state by construction:
`misa-client::prefs` writes a theme, the nodes somebody opened, an unsent draft, and the
counts a picker ranks by. A session is never told, a corrupt document is the defaults rather
than an error, and losing all of it is a client that opens dark with an empty drawer. The
prompt history is deliberately not there: what somebody typed is not a taste.

A panel is where the line is easiest to get wrong, so it is drawn explicitly. The session
owns the _question_: that a credential is being asked for, what the field is called, that it
is a secret, and what the actions mean. The client owns the _answer_: the text in the field
before it is sent (`misa-tui::PanelInput`, the browser's own input), whether the panel takes
the keyboard while it is up, and where on the screen it goes. That is why a session never
sees a draft and why a client cannot invent a panel: `panel.submit` is an action the session
offered, and `panel.close` is one it offered too.

---

## 3. The three stages, and where each stage is enforced

The previous system named the stages `data → presentation data → renderer` and
never landed the third, because all three ran in one process behind one database.
Splitting the agent across a network makes the third stage unavoidable, so the
stages are now a protocol:

| Stage         | What it is                                          | Where it lives                           | Test                                                                              |
| ------------- | --------------------------------------------------- | ---------------------------------------- | --------------------------------------------------------------------------------- |
| **Facts**     | a message, an attempt, a tool result                | `misa-kernel`, `misa-session`'s database | `misa-kernel::tests`                                                              |
| **View tree** | roles, structure, typed fields, stable ids, actions | `misa-session::views`                    | `views::tests`, `misa_proto::view::validate`                                      |
| **Surface**   | cells, DOM, pixels; theme, wrap, scroll, focus      | `misa-render` + each frontend            | `misa-render::lines::tests`, `misa-web::tests` (HTML), `misa-skia::tests` (scene) |

Three tests exist specifically to hold the line:

1. `view::tests::a_tree_is_built_and_valid` — the shipped session's tree satisfies
   every rule a client is entitled to assume, and `Session::read` refuses to send
   one that does not (it reports a fault and keeps the last valid view, which is
   what the previous system learned at its frame boundary).
2. `misa-web::tests::nothing_a_session_sends_reaches_the_page_unescaped` — a tree
   is data written by a model and is escaped on the way into a document.
3. `misa-skia::tests::the_pixel_frontend_decides_colour_and_the_session_did_not` —
   the same tree under two themes produces two appearances, so no colour came from
   the session. Its twin is
   `misa-render::theme::tests::a_role_resolves_to_its_nearest_named_ancestor`:
   a plugin that invents a role is legible in a theme that has never heard of it.

### Why the view tree is built session-side

The alternative — the session emits only domain facts and each client derives its
own view — was rejected because it multiplies presentation policy across every
frontend, and a plugin's UI contribution would then need one implementation per
surface.
The tree is _presentation policy_, and policy lives in the middle layer once — and it is _one_
tree: nothing about a client changes what the session emits, so two clients attached to one session
hold the same nodes, and a change to one of them is addressed by that node's id. The
escape hatch is that a client may ignore the view query entirely: `session.status`,
`session.conversation`, `session.attempts`, and `session.spend` are data
subscriptions, and a client with unusual needs builds its own tree from those.

### Why the client's own loop is small

A client runs a `misa-reframe` loop over `db.ui` — a few handlers (`ui/toggle`,
`ui/input`, `ui/theme`) and a couple of queries. It could have been a plain struct.
It is a loop because the _client's_ state genuinely has the same shape (presentation
state, derived render model, local events), and because making that explicit is what
stops presentation state from leaking back into the session "just this once".

---

## 4. Transport: iroh, and why the protocol is not the transport

`misa_net::Session` is a state machine over messages: it takes a `ClientMsg` and
produces `SessionMsg`s, with no socket in sight. `misa_net::iroh` is the byte
source and sink. `misa-net::tests::the_state_machine_drives_over_channels_with_no_network`
drives the whole thing over two channels.

That split is why the interesting protocol cases are testable at all: a message
before `hello`, a refused protocol version, a client that never attaches, a
subscription whose query does not exist, a faulted intent, a repeated event.

Two update kinds, and the asymmetry is deliberate:

- **Subscriptions** are durable. Re-read when the session's revision changes, and
  only a value that actually changed is sent. A client that reconnects
  re-subscribes and converges. There is no replay protocol because there is nothing
  to replay: the current value _is_ the state.
- **Events** are ephemeral, numbered, and lossy. A streamed token delta, a notice,
  a status line. A client that falls behind drops them and converges on the next
  subscription value. `Session::events` drops an already-delivered seq, so a
  redelivery cannot be applied twice.

Consequences worth naming, because they are the design's cost:

- a client's tree is its own. The canonical view is the whole session, a change to it is
  addressed by node id, and a client applies what it holds — so a client that renders only
  the tail renders the tail, and nothing tells the session a count.
- a control frame is capped at 8 MiB (`misa_proto::MAX_CONTROL_FRAME`), which bounds a
  _message_. Carrying a value of any size is the transport's job — bound the chunk, not the
  value. `Kind::Image` carries a content hash rather than bytes for the same reason it
  always did: the bytes travel on a connection of their own (`misa/blob/0`,
  `misa-net::blob`) rather than through a frame sized for tokens.

### What a change sends

The session's view is a value at a version, and a client's copy of it is its own. Three rules make
that work, and none of them is an optimisation:

- **The canonical view is the whole session.** It is what a client with nothing gets, what a client
  that has fallen too far behind gets, and what a session builds when it starts or resumes. Nothing
  in it depends on who asked, so one tree serves every client.
- **A change is addressed by node id.** A revision's changes go out as one unit with the version, and
  a client applies them to the tree it holds: append this node, set this state, append these spans.
  Applying the ops of revisions R+1…N to the tree at R yields the tree at N, so a client converges by
  applying what it is given, and an op naming something a client has dropped is a no-op for it.
- **In-flight content is a stream, not a tree.** The body of a message still being streamed is in
  neither the tree nor the log: it arrives as a current value and then as appends. A body enters the
  tree when it enters the log, which is where "canonical" stops being ambiguous.

Eviction and staleness follow from the first rule rather than needing machinery of their own: the
session keeps a change chain back to the oldest client it is still serving, a connection's own
bounded queue is what a slow client makes it hold, and a full queue means that client's next read is
the canonical view. That is also the whole reason there is no ACK: the canonical view is the
reconciliation point, and a client whose copy disagrees with it takes it.

### Reserved: the kernel protocol

`misa_proto::ALPN_KERNEL` is declared and unused. The shipped daemon hosts its
sessions in process, and the seam a separately hosted session would need is real
but not needed yet: `Kernel` is a trait, `Runtime::start` takes an `Arc<dyn Kernel>`,
and a daemon-side implementation of the same trait over iroh would let a session run
anywhere. That is the answer to "spin up a session against any of my daemons": the
composition is already detached from the facts, and only the last wire is missing.

---

## 5. The loop: what was kept, what was dropped

Kept from the previous system, and why each one earned its place:

| Rule                                                                         | Where                                                                                   |
| ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Events are the only way to ask for anything; one dispatch is one transaction | `misa_reframe::Loop::dispatch`                                                          |
| Coeffects are the only inputs, so a handler is reproducible                  | `Coeffects`, `Tx::cofx`                                                                 |
| Effects are data, executed after the commit, never during it                 | `Effect`, `Runtime::perform`                                                            |
| An effect the interpreter refuses fails the transaction _before_ it commits  | `Interpreter::accepts`, `tests::an_effect_the_interpreter_refuses_stops_the_commit`     |
| A fault is contained: the transaction rolls back, the session keeps running  | `tests::a_handler_that_faults_takes_its_whole_transaction_with_it`                      |
| Subscriptions declare their inputs; nothing traverses another owner's state  | `misa_reframe::scope`                                                                   |
| State is immutable and shares structure; that is what makes the memo cheap   | `misa-value`, `tests::a_query_recomputes_only_when_an_input_is_a_different_value`       |
| A dispatch chain is bounded and reports instead of hanging                   | `tests::a_dispatch_chain_is_bounded_and_reports_instead_of_hanging`                     |
| An unfinished attempt is a fact, surfaced and never re-issued                | `misa-kernel`'s ledger, `agent::on_cancel`                                              |
| One close-out sentence serves every tool; a tool failure is a result         | `misa_kernel::LocalKernel`'s `ToolRun` arm                                              |
| Every state root declares an owner and a lifetime                            | `misa_session::views::MANIFEST`, `tests::every_root_the_shipped_state_uses_is_declared` |

Dropped, with the reason:

- **The bounded dispatch chain as a _mechanism_.** It survives as a limit. There is
  no global interceptor chain; handlers are declared for an event kind and ordered
  by priority then id. The previous system removed global interception late and
  found it was the right call.
- **Handler-side queries.** A handler reads the database it was given; consumers
  query. The previous system let a handler call `misa.sub` against a speculative
  fork, which made one handler's result depend on another handler's ordering. This
  is a real loss and the trade is recorded in `misa-reframe`'s module docs.
- **Whole-database returns.** Not expressible: `Tx` has no way to replace the root,
  and a root patch must merge.
- **Four patch control sentinels.** Patches are an explicit ordered op list
  (`Set`, `Merge`, `Delete`, `Append`, `AppendAll`), so nothing is inferred from the
  shape of incoming data. Merging is opt-in; appending is opt-in; replacement is the
  default.
- **Cursors for journalling.** With the log as the truth and the fold owning the
  meaning, a _journal_ cursor is not what a client syncs against: a client's cursor is
  a version of the tree, its changes are addressed by node id, and one that has fallen
  past them takes the canonical view. The previous system reached the same conclusion
  about a second representation (its Slices 1 and 2) and this starts there.

### Where the wasm plugins go

A plugin is a wasm component that exports handlers and subscriptions over the same _data_
vocabulary the loop already uses — events, patches, effects, and view nodes — because that is
exactly what `misa-reframe` already passes across a boundary. `wit/policy.wit` is the contract,
`wit/guest/` is the smallest thing that implements it, and `misa-plugin` is the host: it loads a
component, holds what it declares against the composition that would run it, and turns those
declarations into the loop's own `Handler`s and `Subscription`s. Its tests run the whole path
over a real component, so the answer to "does this seam hold?" is a test rather than a diagram.

A plugin presents by returning a tree, and the **session places it in the document** — under a role
it builds from the plugin's id (`plugin.<id>`), with the ids of that subtree prefixed so nothing a
plugin writes can collide with a node the session wrote. That is not the same thing as a
subscription, and the difference is the whole reason it works this way: a subscription is a _pure
function of the database_, which is what makes the loop's memo sound, and a plugin's view is a
function of the database _and_ of what one client can draw. A tree built in a query slot would be
one memo shared between clients that should get different answers; a tree built per client in each
frontend would be presentation policy written five times. So it is built once per client class per
revision, by the layer that owns the document, and every frontend draws it with no frontend code at
all.

Three consequences worth naming, because all three are what make it safe rather than merely
possible:

- **A guest call is bounded.** The engine runs with fuel, and every call into a plugin is given a
  budget. A plugin that loops for ever is a fault in tens of milliseconds — a sentence in place of
  its tree — rather than a session that stops answering, and it costs no threads and no timers.
- **An action is an event.** A plugin that offers an affordance declares it, and receives the
  loop's own `intent/action` event when a client uses it — because that is what an action _is_
  here. There is no router, no id mangling, and nothing the session has to know about who handles
  what: the registry routes by event kind, in priority order, like everything else in this system.
  A plugin may not declare an action the session itself has (`panel.close`), and a tree that offers
  an action its plugin did not declare is refused when it is presented rather than failing in
  somebody's click afterwards.
- **A plugin's state is a fold of its patches.** Its root is neither a kernel fact nor a client's
  presentation, so nothing but the patches themselves can reproduce it: events are ephemeral, and
  what a plugin decided is not derivable from anything else in the log. So the session wraps the
  handlers a composition registered and records every patch into a root that composition declared
  as a `plugin.patch` entry, and a session that resumes the conversation folds them back with the
  same patch application the loop itself uses. No file, no format, and nothing the plugin has to
  cooperate with — and history that cannot be replayed, because the log and the composition
  disagree, is counted and said out loud rather than dropped in silence. The same wrapper is the
  other half of the rule: a handler may only write into the roots its composition declared, so a
  patch into `session.status` is a fault rather than a quiet rewrite of what the loop decided, and
  the declaration is what says which state is a plugin's at all. A plugin's _section_ is
  rebuilt when what it reads changes, because a section declares its inputs the way a subscription
  declares its own — so a plugin that keeps a thousand things costs a rebuild of its own section and
  never a rebuild of the transcript.

---

## 6. What is built, and what is not

Built, with tests, per crate: `misa-value` (21), `misa-proto` (41), `misa-reframe` (24),
`misa-render` (46), `misa-kernel` (96), `misa-session` (88), `misa-net` (29),
`misa-plugin` (16, and thirteen more behind `--features guest-fixture`), `misa-client` (61),
`misa-tui` (27), `misa-web` (20), `misa-daemon` (4), `misa-skia` (5), and one more behind
`misa-skia --features paint`. That is 478 tests and no skips: `cargo test --workspace` is the
gate, and these numbers are read back from it rather than remembered.

The largest thing stated in this document that is _not_ built is the view's change protocol: today a
revision re-sends whole subscription values (`Session::refresh` compares what it last sent against
what it holds), a streamed body is in the tree as well as in an event, and a session still asks a
client how much history to send. The op stream, the content stream and the incremental view that
makes producing ops O(change) are `plan.md` phases 2 and 3, and the crate slice in §1 is phase 4.

Three of those crates exist because of what a _client_ needs and not because of what a
session does: `misa-client` is the picker, the editor, and the selection, `misa-render` is how a
tree becomes text and a fact becomes words, and `misa-kernel` is where every capability lives —
the log and the ledger on disk, HTTP, credentials, blobs, providers, tools, and search. The
kernel is the largest of them, which is the shape the architecture predicts: facts and capability
are what cannot be policy.

The end-to-end claim that is actually tested: a prompt recorded, a scripted provider
streaming into a placeholder node, a tool call returned, executed, recorded, and the
loop continued — with the _view_ the client receives asserted as text
(`misa-session::tests::a_tool_call_round_trips_and_the_turn_finishes`) and the
transport's incremental behaviour asserted over channels.

Three of those claims are also checked by hand, between two processes, because that is the
only way to know the pieces meet: `misa-daemon --no-relay` beside `misa` reaches a session
over a real endpoint (a ticket, a QUIC stream, a rendered transcript); `misa-web` attached to
the same daemon takes a file from a browser's form, puts it in the daemon's store over
`/misa/blob/0`, shows the transcript's `<img src="/blob/…">`, and serves those bytes back with
`cache-control: immutable`; and a name that is not a content hash is refused before the store
is asked. The client binds with a relay only when the ticket has no direct address, because a
daemon on a machine with no route out is otherwise unreachable by the clients shipped beside
it.

Not built, in the order they matter. `docs/parity.md` is the full matrix; these are the
items with no code at all, plus the two that are structural.

1. **Saving on the remaining surfaces.** Terminal `/save [number] <local path>` and browser
   Save attachment actions request a kernel-confirmed file offer and keep the received bytes.
   The protocol routes each answer only to the requesting connection; the destination stays
   client-local. Android and pixel clients still need their own destination pickers.
2. **A window** in the pixel frontend. The scene, the raster, and the PNG are done; a window
   is a second consumer of the scene and needs nothing from a session.
3. **The same memory for the browser and the phone.** `misa-client::prefs` is the terminal's
   today: a surface that renders server-side has to decide whose state a page's draft is,
   which is a question about browsers rather than about this architecture.

Five things are honest limitations rather than planned work:

- **`misa-skia`'s text flow is the linear renderer's.** The scene maps roles to
  appearance and is tested, but it wraps to a column count and stacks runs. A pixel
  frontend that wants proportional type or a non-linear layout needs its own layout;
  needs its own layout. The raster is behind a `paint` feature today only because Skia is not
  yet an input the build provides; `plan.md` phase 7 builds the pixel frontend with the Skia it
  needs, unconditionally, because a pixel frontend that cannot paint is not a frontend.
- **A window does not exist in any frontend.** `misa-skia` renders a PNG. The
  scene is the part worth getting right first.
- **A session still tells a client how much history to send, and still reads what a client can
  draw.** The window argument on the view query is the last place a client hands a session a
  count, and `graphics` and `native_details` are the last things a session reads about a
  surface. `plan.md` phases 1 and 2 delete them — the first with the class a view is memoised
  by, the second when the transport can carry a value of any size.
- **Device authorization is session-scoped.** Any attached client can use the panel's Cancel
  authorization action. The kernel stops the named flow owned by that session and reports its
  outcome; dismissing a panel alone keeps the flow running.

---

## 7. Open questions

These are decisions not yet made, stated so they are not mistaken for decisions
already made.

1. **Where the plugin _effects_ stop.** A plugin may express any effect kind the
   session's interpreter accepts. Should a plugin be able to _extend_ the accepted
   set (by registering an interpreter arm) or only use it? Extending is more
   powerful and makes the trust boundary a policy rather than a property. The
   current answer is "only use it", because that is what `Interpreter::accepts` can
   enforce.
2. **How far a session should go for a second client.** Reachable by more than one at once is
   settled — it is built, and
   `misa-net::server::tests::two_clients_on_one_session_converge_on_the_same_transcript` holds
   it. What is not decided is what a _client_ should do when it sees that somebody else is
   driving: today two clients share a transcript and either may prompt, and a "somebody else is
   here" indicator is not written.
3. **How a session is addressed when it outlives its daemon.** A ticket is
   `misa:<endpoint id>:<session>`, so it names a process. A session that can move
   needs a name that is not a process, and that is the same question as the kernel
   protocol in §4.
4. **Whether `Intent::Action` should be typed.** It is a string plus fields, which
   is open and lets a plugin invent an affordance without a protocol change. It also
   means a client cannot tell a valid action from a typo before sending it. The
   session advertises its queries; it could advertise its actions.

---

## 8. Rules for changing this

- A new fact goes in the kernel. A new decision goes in the session. A new
  appearance goes in a client. If a change needs two of those, it is two changes.
- Nothing may enter `misa-proto::view` that a client could decide. The test for that
  is: _could two clients disagree about the right answer and both be right?_ A
  colour, yes. A role, no. A state, no.
- Nothing may enter `Intent` that a session would have to trust. The test is: _could
  a client break the agent with this?_ If yes, it is not a client message.
- A state root must be declared: the shipped session's in `views::MANIFEST`, with an owner and a
  lifetime, and a composition's by the `Contribution` that adds it. Both halves of that are
  enforced, and they are enforced in the two places the mistake can be made: a name the session
  already owns cannot be _claimed_ (`with_root`, against the manifest), and a handler may only
  _write_ into the roots its composition declared (the wrapper `registry` puts on every handler).
  The second is the one a declaration cannot catch, and it is also what makes a patch recordable —
  and therefore replayable (§5). A presentation root that is journalled is a bug the manifest test
  catches.
- A fault is data. A handler that fails rolls back and reports; a session that
  cannot build a view keeps the last valid one. Nothing in this system panics on
  input, and `Session::read` is where that is enforced for the wire.

Provider usage is the named `kernel.usage` capability: the kernel owns the endpoint, credential slot and normalization; `/usage` carries scalar facts with semantic roles. Refresh requests coalesce, stale completions are ignored, and failed refreshes clear quota facts. `kernel.blob.file` remains a named local attachment capability: its path means a file on the daemon, just as a tool file read does. It does not grant policy a general HTTP effect or let a client name a state path; `/attach` deliberately authorizes that local read.
