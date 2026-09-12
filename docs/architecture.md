# Architecture position and migration plan

This is a position plus a plan, not a description of the current code. The
current contract is documented in the README and generated in
`docs/catalogs.md`/`docs/services.md`; the dated reasoning for each landed slice
belongs in `docs/history/architecture-audit.md`, as previous work does.

Baseline evidence for the plan:

```sh
zig build test --summary all        # 234/234 steps, 312/316 tests, 4 grammar-dependent skips
sh tools/fennel tests/conversation-state.fnl
sh tools/fennel tools/generate-docs.fnl --check
```

The suite runs the generated-document check in `--check` mode, and the
`githooks/pre-commit` hook refuses a partially staged file, so every slice below
states how it is verified rather than assuming the suite will cover it.

## 1. Position

A sealed system map composes four layers. Seams are keys, not imports. Effects
and views are data with interpreters. Policies own no state; the kernel
remembers; presentation owns only presentation state. Everything a plugin may
add is a declaration in a named catalog.

Two existing systems inform this and one place deliberately diverges:

- **re-frame** gives the loop and the data-oriented stance. Events are data, and
  the registered handlers collectively are the virtual machine that executes
  them. Handlers take coeffects — the state of the world, as data, as presented
  to them — and return effects as data. Views are hiccup: data plus an
  interpreter. So rendering is not a special case, it is `data → presentation
data → renderer`.
- **Cordis** (the plugin framework under DeepSeek Harness) gives composition.
  Every part of that product is a plugin, including the model adapter, the tool
  registry, the session log, and the agent loop, so each is replaceable from
  configuration and there is no privileged core to patch. Services claim a
  stable `ctx.<key>`; `inject` declares requirements so load order is a graph
  rather than boot sequencing; events carry a declared dispatch mode;
  registrations are reversible effects with disposers; a running application is
  a plugin tree composed at boot from ordered layers with patch overlays. It
  keeps durable replay facts on its session event log and live control state on
  separate agent events, deriving the request from the log rather than storing
  it. It also names its seams explicitly — session persistence, LLM adapter,
  settings, credentials, attachments — each with swappable implementations, and
  packages a service can be a core spine service, a swappable capability seam,
  or a composition point.
- **Divergence.** Cordis mounts plugins at runtime and unwinds them on reload.
  We keep the install-once, sealed, validated, serializable map, because our own
  contract says runtime effects cannot add definitions or reopen installation,
  and because sealing is what makes the whole application validatable up front
  and JSON-able across the native boundary. Development reload stays "rebuild
  the map and restart the session".

We take Cordis's keys, requirements, layers, patches, seams, contribution
pipelines, and package-owned invariants. We do not take its mutable runtime.

## 2. Layers and where things live

| Layer                             | Current location                                                                                                                  | Owns                                                                                                              | State                      | Swapped by                                                                                 |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------ |
| Interpreter                       | `src/session/*`, `src/capability/*`, `src/lua_runtime/framework.fnl`                                                              | transaction commit, effect validation and execution, operation owner, cancellation, patches, subscriptions, views | none                       | rebuild                                                                                    |
| Kernel (session)                  | `src/conversation/*`, `extensions/misa/{agent,conversation,tools}`                                                                | canonical history, the open step, the attempt ledger, the log, tool registry and execution pipeline               | `db.session.*` and the log | service keys: `session.log`, `session.loop`, `session.tools`                               |
| Policies                          | `extensions/misa/{compaction,models,costs,usage,editor/queue}`                                                                    | what and when: compaction, selection, retry, accounting, queueing, rendering policy                               | none                       | catalog entries                                                                            |
| Adapters, capabilities, and seams | `src/{http,auth,terminal,image,syntax}`, `extensions/misa/{providers,protocols,search}`                                           | how and where: transports, protocols, credential stores, settings stores, terminal, decoding                      | none in `db`               | effect catalog plus seam keys: `llm`, `settings`, `credentials`, `session.log`, `frontend` |
| Presentation                      | `extensions/misa/{ui,transcript,editor,choices,dialogs,selection,markdown,links}`, `actions.fnl`, `keybindings.fnl`, `commands/*` | render models, component resolution, input routing and policies                                                   | `db.ui.*`                  | component, theme, and layer catalogs; the `frontend` seam                                  |
| Composition                       | `config/*.fnl`, `app.include`, `misa.snapshot`, `misa.patch`                                                                      | the system map itself                                                                                             | none                       | profiles, bundles, patches                                                                 |

The direction of dependency is one way: the kernel knows nothing above it;
policies read kernel projections and emit kernel requests; presentation reads
everything and writes only itself.

## 3. The boundary between Zig and Lua

Zig owns a concern when it fails one of four tests:

1. **Trust** — it must hold even if a plugin is hostile: credentials, permitted
   origins, filesystem scope, terminal control.
2. **Durability and versioning** — it must be enforced for every consumer and
   must evolve with `payload_version`: the log, its fold, the attempt ledger,
   budgets, effect validation.
3. **Replay** — it must survive a crash: the facts, what they mean, and what they
   leave unresolved.
4. **Capability and hot paths** — OS access, transport streaming, geometry,
   decoding.

Everything else stays in Lua. Every crossing is data: an effect from a closed,
validated union, or a validated completion event or service call. There is no
FFI into native code, no function pointer into policy, and no shared mutable
state. That constraint is what keeps this a boundary rather than a rewrite.

Explicitly Lua, and not candidates for moving: the turn loop, provider request
and stream shaping, tool declarations and translators, every presentation
concern, patch application, subscription evaluation, input routing, Markdown
parsing, and layout.

Explicitly Zig and already right: the event loop and effect validation
(`src/session`), process and file capabilities, HTTP transport, credentials and
OAuth, the state file, the SQLite log, terminal and image handling, and syntax
highlighting workers.

## 4. State: ownership and lifetime

Two independent axes, declared per state path:

- **Ownership** — kernel, policy (never), adapter (never in `db`), presentation,
  or external (durable, outside the log).
- **Lifetime** — `log` (durable facts), `fold` (pure projection), `observation`
  (world-derived, carries a check), `ephemeral` (in-flight and interaction
  state), `external` (preferences, credentials, blobs).

A policy owns no state. State a policy seems to need is either a kernel fact or
a derived check. Applied literally, this is what retires `db.compaction`: its
`sequence` is a kernel fact (one attempt ledger), its `signature` is a derived
comparison (the log tail the attempt started at), its `active` record is kernel
in-flight work, its dialog is presentation, and its trigger, prompt, and bounds
are configuration.

Observations carry `{value, at_ms, source}` and a named check, or are explicitly
unknowable. A missing check never means "still fresh". The existing anchored-edit
contract is the reference implementation: `snapshot` plus `LINE#HASH` anchors
either match or fail the effect as `StaleLineAnchor`/`StaleSnapshotReadFileAgain`.

Invariants, each with where it is enforced:

| Invariant                                                         | Enforced by                                               |
| ----------------------------------------------------------------- | --------------------------------------------------------- |
| Only the kernel writes the log; facts append, never rewrite       | the store's insert-only statements                        |
| Nothing journaled is derived from ephemeral or presentation state | install-time manifest check plus an effect-time assertion |
| Policies hold no state                                            | manifest: a policy module may not declare a namespace     |
| Presentation patches only its own root                            | install-time path rule and a patch-time assertion         |
| Every state path declares ownership and lifetime                  | install-time manifest validation                          |
| Observations carry a check or are unknowable                      | a check catalog consulted at use sites                    |
| Canonical history always satisfies the provider contract          | the native fold and message-shape validation              |
| Loading a conversation performs no effect                         | the load path emits dispatches only                       |

## 5. Composition: the system map

Composition is already data: `config/*.fnl` returns `{config, definitions}`,
`misa.snapshot` copies, `app.include` merges fragments, and `misa.patch` overlays
configuration. What the map needs to become a complete answer to "swap an
implementation by constructing a different map":

- **Named layers.** Profiles (a named composition) over bundles (patchable
  config rows plus the code they mount), with a user patch overlay applied last.
  Today `config/default.fnl` is the single profile and the integration configs
  are bespoke trees; naming the mechanism changes little and makes the tree
  diffable.
- **Seam keys.** Today services exist and `requirements` is validated at install.
  Extend it so every cross-module capability has a key: `session.log`, `llm`,
  `settings`, `credentials`, `attachments`, `frontend`, `session.tools`.
- **Contribution pipelines.** Today two artifacts are assembled monolithically:
  the system prompt is one `config.agent.system_prompt` string, and the tool set
  is one `misa.tools.all` call (`src/lua_runtime/framework.fnl`), both read
  directly in the agent request path.
  Anything many extensions contribute to — prompt sections, tool schemas,
  request headers and options — becomes an ordered, declared-input pipeline
  (Cordis's waterfall, re-frame's interceptor chain, without the global
  interception this tree deliberately removed). Routes choose an outcome;
  pipelines assemble an artifact.

Extension surface, by catalog, and the layer each belongs to:

| Catalog                                                                                      | Purpose                                | Layer                         |
| -------------------------------------------------------------------------------------------- | -------------------------------------- | ----------------------------- |
| `events`, `routes`, `coeffects`                                                              | behaviour and reactions                | any                           |
| `effects`, `services`, `requirements`                                                        | capability seams and pure services     | kernel, adapter               |
| `subscriptions`, `projections`, `views`, `view-layers`, `components`, `themes`, `animations` | presentation data and its interpreters | presentation                  |
| `tools`, `models`, `auth-providers`, `search-backends`, `serializers`, `protocols`           | declarations                           | policy, adapter               |
| `commands`, `actions`, `keybindings`, `completions`, `validators`                            | interaction policy                     | presentation                  |
| `pipelines` (new)                                                                            | ordered contribution to one artifact   | kernel or policy, by artifact |

## 6. Mechanics

- **Events** are the only way to ask for anything: one dispatch, one transaction,
  a patch plus ordered effects. Intents only.
- **Coeffects** are the only inputs. Every nondeterministic or external value
  enters here: clock, argv, config, terminal facts, derived queries. This is what
  makes policy pure and replay possible.
- **Effects** are data, executed after commit by the native interpreter. No IO in
  handlers; a handler that needs an answer emits an effect and handles its
  completion event.
- **Subscriptions and projections** are pure, declared-input queries. Consumers
  compose them rather than traversing another owner's state.
- **Views** return semantic lines, spans, styles, roles, actions, links, and
  animations — presentation data, not terminal output.
- **The renderer** interprets presentation data: the native presenter today; a
  web or plain frontend later. Stage boundaries are `data → presentation data →
renderer`, and only the third stage is frontend-specific.

## 7. Scenarios these must support

**Subagents.** A tool (`misa.tools.subagent`) plus a driver of `session.loop` for
a child conversation, with scoped registrations so a child can add tools or
prompt sections without leaking. The kernel supplies per-conversation session
state; the log already stores multiple conversations with fork provenance; the
attempt ledger's `parent_request_id` already groups a fanned-out turn for cost.
Visibility of the child transcript is presentation policy.

**A different frontend.** Implements stage three: a renderer, an input decoder,
and its component and layer set, consuming the same presentation data and
dispatching the same events. It replaces the terminal driver behind the
`frontend` seam and owns its own `db.ui.*` state. Prerequisites: the `frontend`
seam, and splitting the transcript render model out of the kernel into
presentation.

**A bespoke personal tool.** A declaration, an effect translator, a completion
translator, a presentation binding, and an argument schema. It expresses itself
over
existing effect kinds — `process/run`, `http/request`, `file/*`, `json/decode`,
`clipboard`, `image/*` — so most tools need no native change; a genuinely new OS
capability is a new effect kind and adapter. Its result appends as a message; it
never writes presentation state or the log directly.

**An extension that changes the UI.** Component entries for existing or new
roles, view layers (dock, overlay, exclusive), themes, animations, and actions or
keybindings that dispatch events. It never writes kernel state, never derives a
fact from presentation state, and never reads another feature's private state
where a declared projection exists.

## 8. Migration plan

Each slice is independently landable, states its verification, and notes its
risk. Order is deliberate: the kernel primitives dictate the shape of everything
above them.

Progress: Slice 3's storage half has landed — `conversation/request` records and
enriches one attempt in `provider_requests`, named by the branch that issued it,
and `conversation/load` returns a branch's attempts beside its entries — but the
half that matters for the plan is still missing: nothing writes a start
automatically, so a model call can still happen without a row. That half belongs
inside the call itself, which is the next step. Slice 0 has landed
(`extensions/misa/standard/state.fnl`) as a declared
manifest with an install validator and a test that pins the assignments this
document commits to; it is a checklist, not enforcement, because a root nobody
declares still passes unnoticed. Slice 1's validation half has landed
(`src/conversation/message.zig`, recorded in `docs/history/architecture-audit.md`)
— the log now refuses a message
a provider would reject, and refuses a tool result that answers no open call. The
fold itself still lives in `extensions/misa/conversation.fnl`. Slice 2's cursor
deletion turns out to depend on Slice 4 (and really Slice 6): the cursor exists
because canonical history and the log are two representations, so it can only go
once history _is_ the fold that the kernel owns. The remaining order is unchanged.

### Slice 0 — manifest and invariants (test-only)

Add install-time validation in `src/lua_runtime/framework.fnl` for the state
manifest: every namespace declared in an application's state must carry
ownership and lifetime, and the declared root must match the layer the module
claims. Add the new invariant tests as standalone Fennel contracts:
`tests/state-manifest.fnl` and `tests/effect-boundary.fnl`.

Verify: `sh tools/fennel tests/state-manifest.fnl`, plus the existing suite. No
behaviour change; a missing declaration fails only in the new tests until the
roots are renamed in Slice 10.

Risk: low. This is the scaffolding that makes later slices checkable.

### Slice 1 — the fold, in Zig

Move "what a stored conversation means" from `extensions/misa/conversation.fnl`
(`loaded`, `install`, `closed-history`, `interrupted-results`) into
`src/conversation/`: canonical history assembly, `reset` handling, ordering and
role invariants, the unanswered-call closure, and `provider_state` carriage.
Expose `history(conversation)` and `unresolved(conversation)`. Validate the
canonical message shape natively: roles, block kinds, tool-call and tool-result
pairing, so no plugin can write history a provider will reject. Legacy databases
keep loading through the existing migration path.

Verify: new native tests for every fold rule, including each crash window (tail
is a user message, tail is an assistant message with unanswered calls, tail is
part of a tool batch, tail after a reset); the existing legacy-upgrade test;
`tests/conversation-state.fnl` re-pointed at the native fold. The closure it
derives uses the single wording Section 10 settles, and reports what it closed
rather than only the count, so Slice 3 can surface an unfinished attempt the same
way.

Risk: medium. It is versioned durability semantics; it needs a migration-safe
reading path and it invalidates the Lua reassembly the journal depends on, so
Slice 2 lands immediately after.

### Slice 2 — delete the journal cursor

With history being the fold, `conversation.synced`/`pending` and the
one-append-at-a-time chain have nothing to reconcile. `extensions/misa/
conversation.fnl` shrinks to configuration (id, label, list bound) and `/resume`
UX (list, pick, load). Appending becomes "record this fact", and the kernel
publishes "the log advanced" instead of each patch site publishing
`agent/history-changed`.

Verify: `tests/conversation-state.fnl` reduced to the configuration, picker, and
resume contracts; the integration cases in `tests/integration/storage.zig` keep
their behaviour — one record per settled message, in order — even though the
code that issues those appends moves from the journal to the loop.

Risk: medium. This removes the mechanism that made the tail durable per message;
Slice 1's fold and Slice 3's ledger must be in place so nothing regresses.

### Slice 3 — the attempt ledger

Wire the existing (currently unreachable) `provider_requests` record:
`conversation/append` or a dedicated effect accepts attempt facts, and outcomes
link to messages. Cost and usage stop living in session memory: `db.costs` and
`db.usage` become subscriptions over ledger rows plus configuration, keeping the
indicator and dashboard output identical.

An unfinished response becomes a fact here: an attempt with no outcome is what
lets the fold report "a request was made and nothing came back" instead of
mistaking the state for a turn about to start. It is surfaced, never re-issued
(Section 10).

Verify: native tests for attempt insert, enrichment, retry grouping under
`parent_request_id`, message linking, and concurrent writers; Fennel tests for the
cost and usage projections; integration case for `/usage` and the cost
indicator.

Risk: medium-high. It changes cost accounting from ephemeral to durable; without
the projection matching, `/usage` output changes. Land the projection in the same
slice.

### Slice 4 — kernel session state, side request, history replacement

Kernel primitives:

- per-conversation session state (history, open step, in-flight calls) keyed by
  conversation id, replacing the singleton `db.agent` durable fields. Only the
  ephemeral view of the current step remains in Lua.
- `side request`: a model call that is not a turn, with its own attempt row and
  cancellation, usable by any policy.
- `replace history`: an explicit operation that records the replacement, with
  validity expressed as a kernel check (the tail is unchanged since the attempt).

Verify: native tests for the conversation registry, cancellation scoping, and
replacement validity; a Fennel contract driving two conversations at once.

Risk: high. This is the largest data-model change and it touches every reader of
`db.agent`; do it behind the Slice 6 rewrite, in one step per consumer.

### Slice 5 — compaction as the worked example

Rewrite `extensions/misa/compaction.fnl` as policy on top of Slice 4: trigger
from the token meter and threshold, `side request` for the summary, `replace
history` guarded by the tail check. Delete `db.compaction`. Move the progress
dialog to presentation.

Verify: `tests/compaction.fnl` and `tests/compaction-cascade.fnl` rewritten
against the new primitives, including the discarded/stale/failed outcomes and
the "already attempted" rule derived from the ledger rather than a stored
signature.

Risk: medium. It is the first real consumer of the new primitives, so it should
land before other policies migrate.

### Slice 6 — split the agent loop

`extensions/misa/agent/init.fnl` becomes three modules: declarations (tools,
options, prompt sections), turn policy (continuation after tools, retry,
cancellation intent, usage), and step submission (assemble the request from the
kernel fold plus policies, emit the provider effect, record the attempt). It
gives up `messages`, `request_seq`, `active_request_id`, `pending_tools`,
`tool_batch`, `status`, and `exit_after_response`; `stream` remains explicitly
ephemeral. The loop stays in Lua — porting it natively would need a callback
across the VM boundary per step and would drag policy native.

Verify: `tests/agent-stream-state.fnl` and the transcript/streaming integration
cases, plus a new contract that the loop's only writes to durable state are
kernel requests.

Risk: high. Largest single rewrite; it should be a pure move plus deletions with
no behaviour change in the same commit.

### Slice 7 — queue and editor placement

Queueing becomes kernel scheduling (a queued prompt is session intent); the dock
stays presentation. Editor, choices, dialogs, selection, and hover move under
`db.ui.*` with their lifetime declared ephemeral.

Verify: `tests/editor-lifecycle.fnl`, `tests/editing-state.fnl`,
`tests/control-ownership.fnl`, and the PTY regressions.

Risk: low-medium, mostly mechanical, but it touches input routing.

### Slice 8 — contribution pipelines

Add the `pipelines` catalog and use it for the three monolithic assemblies:
prompt sections, tool schemas, and request options/headers. Ordering is by
priority then ID, contributors receive the accumulated artifact and return a new
one, and every pipeline's output is validated before it reaches an effect.

Verify: new Fennel contracts for ordering, validation, and rejection; provider
request-body tests in `tests/integration/providers.zig` unchanged in content.

Risk: medium. Every provider's request body flows through it, so the fixtures are
the safety net.

### Slice 9 — composition names

Name profiles and bundles: a profile is a named list of bundles plus patch
overlays; ship `default`, `headless`, and `plain`. `config/default.fnl` becomes
a bundle; `tests/integration/configs/*.fnl` become test profiles.

Verify: the doc generator still matches, the fixture application boots from each
profile, and a new case asserts that a patch overlay replaces one row.

Risk: low.

### Slice 10 — frontend seam and the presentation split

Extract the transcript render model from the kernel: the kernel exposes a
projection of the log, presentation owns the model that is rendered. Declare the
`frontend` seam (presentation data in, input events out) with the native TUI as
its first implementation, and rename presentation namespaces under `db.ui`.

Verify: the existing PTY suite against the TUI implementation, plus a smoke
frontend that consumes presentation data and renders plain text, proving the
seam is sufficient without forking the transcript.

Risk: medium-high. It is the change a second frontend needs, and it is the one
most likely to reveal a kernel assumption that leaked into presentation.

## 9. Non-goals

- No live plugin mounting, hot reload, or disposer machinery. The map is sealed
  and validated at install; development reload is rebuild-and-restart.
- No port of the turn loop to Zig. The kernel owns facts, the fold, the ledger,
  the registry, and validation; policy owns decisions.
- No port of Markdown, layout, components, or query evaluation to Zig.
- No second source of truth: while the fold lives in Zig, Lua holds only
  projections and ephemeral state. A Lua-side history cache would recreate the
  reconciliation problem this plan removes.

## 10. Decisions and open questions

### Decided

Nothing unfinished is ever resumed automatically, and nothing is classified. The
three questions this section previously carried are answered by one rule: when a
step has no recorded outcome, the log says so and the session stops there.

- **An unfinished response is not re-issued.** Loading a conversation performs
  no effect. The unfinished attempt is a fact (once Slice 3 records attempts),
  the fold reports it, and the session surfaces it so a person decides. No
  automatic spend, no automatic retry, on restart or on `/resume`.
- **One close-out sentence serves every tool.** An unanswered call is closed with
  the same text and `is_error = true`: the call started, its result was never
  recorded, so whether it took effect is unknown, and what that means depends on
  the tool. Classifying tools to phrase it better is a losing game — `ls` through
  the shell is semantically `list_directory` — so the model is told what happened
  and re-checks what it depends on.
- **No tool declares itself retry-safe.** Nothing re-runs on load; `read_file` is
  not special-cased and no effect-class field is added to a declaration.

The property this buys is worth stating plainly: resuming folds, closes out, and
reports. It spends nothing, touches nothing, and cannot change the world.

### Open

1. The prompt input bound: a limit on attachments and prompt bytes, so the
   record budget is provably unreachable. This is an input policy and it should
   be a decision, not an accident.
2. Whether a resumed interrupted conversation silently branches (new id with
   fork provenance) or asks.
3. Timing of Slice 9: profiles and bundles are cheap but they rename things
   users see in configuration, so they can wait until the boundary work is done.

## 11. Verification

- `zig build test --summary all` is the gate for every slice.
- Standalone Fennel contracts for policy and presentation behaviour:
  `sh tools/fennel tests/<name>.fnl`.
- `sh tools/fennel tools/generate-docs.fnl --check` after any catalog change.
- The treefmt pre-commit hook; it refuses partial stages, so slices commit whole
  files.
- PTY and integration cases for terminal, frontend, and provider-visible
  behaviour.
- Migration fixtures for the fold: a database written by the previous version
  must load with the same history, including interrupt windows.
