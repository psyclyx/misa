# Dated architecture audit and implementation history

Notes below were written as the work landed, in order, and later entries
supersede earlier ones. They are retained for the reasoning and the
verification they record, not as a description of the current code.

### The log refuses history a provider would reject (2026-09-11)

The `message` kind's shape was a Lua convention. `misa.agent` built canonical
messages and `misa.conversation` appended them, and nothing checked the shape on
the way in, so a bug — or a plugin — could write a transcript that failed only
later, at the next request. The store's own header even called the payload
opaque, which was true of the bytes and false of the contract.

`src/conversation/message.zig` now owns that contract: the roles, the block kinds
and the fields a provider needs from each (`text` and `thinking` carry text,
`tool_call` carries a nonempty id and name plus an object of arguments, `image`
carries a base64 source with a media type and data), and a `tool` message must
carry the call it answers and a boolean verdict. `append` refuses anything else
as `InvalidMessageShape` before it opens a transaction.

Pairing is checked in one direction as well: a tool result must answer a call the
conversation has left open, and a call is answered at most once, so a duplicated
or misordered batch fails as `InvalidToolPairing`. The tail is read inside the
write transaction, `reset` discards what a branch left open, and the scan is
bounded — a tail longer than the window leaves pairing unchecked rather than
rejecting a legal append. A transcript that _ends_ with open calls stays legal,
because that is what a crash mid-batch looks like, and the fold closes it on load.

Evidence: `src/conversation/message.zig` tests every accepted and refused shape;
the store test "a tool result must answer a call the conversation left open"
covers unknown ids, a repeated answer, the legal sequence, a reset, per
conversation isolation, and the crash window; the store's own fixtures and the
integration conversation fixture now append canonical messages.

### Recording is not where a bound belongs (2026-09-11)

Two different refusals were coming out of the store under one name. The first was
a real bug: the 64 KiB budget for the conversation _header_ document — the small
index of labels and fork provenance the store composes — was also applied to
recorded messages, by `validateDocument` on every key and string of an entry and
by `encodeDocument` on its canonical payload. A tool result, a pasted attachment,
or a compaction handoff larger than that was therefore refused with
`DocumentTooLarge` after a turn had already been accepted, and the store's own
`max_entry_bytes` (1 MiB) check could never run because encoding had already
failed. Because the encoding counts escaped bytes and validation counts raw ones,
a payload under the budget could even fail after validation.

The second was a design fault. The journal sent a whole turn in one
`conversation/append`, so a long turn ran into the effect's entry-count limit
(256) or the batch byte limit (4 MiB) and failed whole — and then reported and
skipped the range, because the cursor only advances on a completion. The previous
attempt capped the journal's own batches to avoid that, which is a limit being
designed around rather than removed.

Now the store keeps two named budgets that answer different questions. A message
is bounded by `max_message_bytes`, a backstop far above any part the application
can acquire (an image attachment is at most an 8 MiB source, which becomes ~10.7
MiB of base64 plus a ~0.8 MiB preview; a file read and each captured stream are
at most 1 MiB). The header keeps `max_metadata_bytes`, and a refusal names the
budget it hit (`EntryTooLarge`, `MetadataTooLarge`, `RequestTooLarge`). The batch
byte limit is gone: a per-entry budget and the entry-count backstop already bound
a transaction, and providers of a batch are the only thing that could reach them.

The journal then stops batching altogether. The agent owner publishes
`agent/history-changed` wherever canonical history changes — the submitted
prompt, a finished response, a completed tool batch, a cancellation, a reset, a
resume, and compaction's handoff — and the conversation owner appends exactly the
one message after its cursor, continuing from each completion. A whole message is
the finest unit available, because canonical history only grows in messages
(recording a partial assistant response would assert something false), so the
turn is now durable as it happens rather than at the end, an append carries one
entry by construction, and a crash loses at most the message in flight. The
turn-settled handler is kept as a catch-up for a change that arrives without a
signal.

Evidence: the native store test "a message budget is separate from the header
metadata budget" records a 100 KiB message that the previous code refused and
pins both budget failures and the removed batch limit; `tests/conversation-state.fnl`
walks a 300-message history through 300 single-entry appends; and
`tests/integration/storage.zig` runs one turn whose history needs 259 messages,
asserting `recorded 259 in 259 appends` with no effect or recording error.

### The persisted model wins the first request (2026-09-11)

`config.models.default` was reaching the first request of a session. The models
extension patched `selected` from the configured default at `app/start` and only
replaced it when the `model-selection` load reported a model that the current
catalogue already offered. Discovery registers a provider's models later, and the
load is a native operation whose completion competes with the queued startup
prompt, so a session could start on, or fall back to, the default after the
configured default itself became unavailable.

`db.models` now distinguishes `preferred` — the model the user last chose, which
outlives a catalogue that does not offer it yet — from `selected`, the offered
model requests use. `rebuild` resolves `selected`, then `preferred`, and only
then the configured default, so a provider that registers its catalogue later
still restores the saved choice. `model/select` and `model/open` record
`preferred` with the selection. The load's completion clears
`db.models.selection_pending`, which holds a queued prompt (and an early
`agent/submit`) back until the selection is known, and publishes
`models/selection-settled` so the agent continues after the model owner's patch
rather than racing it: handlers for one event run in ascending priority, so a
continuation cannot be ordered by priority alone. EOF no longer discards that
queued turn, and the readiness message names `preferred` instead of the
configured default.

Evidence: `tests/model-startup.fnl` covers the configured default, a persisted
selection that replaces it, and a saved model whose provider arrives later;
`tests/model-affordances.fnl` pins the load handler's patch and settled effect;
`tests/agent-stream-state.fnl` and `tests/editing-state.fnl` cover the queued
prompt gate and EOF; `tests/integration/providers.zig` starts a fixture session
from a seeded state file and asserts the request uses the saved model.

### Single selected model replaces role assignments (2026-09-10)

Model roles are removed. `misa.models` owns exactly one active selection
(`db.models.selected`), and every consumer resolves it through the same
`models/selected` projection: the conversation request, request options, the
status indicator, and `misa.compaction`. The `/role` command, the `model/role`
and `model/roles-loaded` events, the `models.for-role` service, `models.roles`,
and `config.models.roles` are gone. `config.models.default` remains only as the
initial value when nothing is persisted; `rebuild` takes it as its sole fallback.

The selection persists under a dedicated `model-selection` namespace
(`{:selected id}`): `model/select` and `model/open` save it, `app/start` loads
it, and `model/selection-loaded` restores a still-offered model. This replaces
the overloaded `model-roles` record and the role persistence it carried; old
`model-roles` data is ignored rather than migrated.

Summarization is now a single, explicit feature. `misa.compaction` runs its
tool-free handoff request on the selected model, and its automatic trigger
requires an available selected model that declares a `context_window`; settings
no longer carry a role. The separate tool-result summary extension
(`misa.transcript.tools.summary`), its `tool_summary` state, the
`transcript/tool-summary` presentation and `model.summarize-tool` handler, the
costs handler, and the tool-block summary render path are removed; the
transcript keeps only the collapsed-error first-line preview. This supersedes the
background-model-role compaction design above.

Evidence: `tests/model-affordances.fnl` covers the persisted `model-selection`
round trip, the `/model` command path, and that a vanished saved model is
ignored; `tests/compaction.fnl` and `tests/compaction-cascade.fnl` pin the
selected-model trigger, its inert cases (no model, no context window), and the
`/compact` refusal; `tests/transcript-stream-interleave.fnl` keeps the
block-window regression through compaction on the shared model;
`tests/cost-state.fnl` exercises background usage accounting through
`compaction/usage`; `tests/model-role-input.fnl` and `tests/tool-summary.fnl` are
removed, and `tests/extension-style.fnl` audits the updated extension graph.

### Transcript block windows under concurrent messages (2026-09-10)

`misa.transcript.model` keeps one contiguous window of `db.messages.blocks` per
response (`block_start`, `block_count`), and every scoped read — `find-block` for
stream deltas, the `transcript.blocks` service used by presentation and selection —
resolves blocks inside that window. A response's next block was appended at the end
of the array, which only holds while nothing else arrives during the stream. A
standalone message can: a harness notice, a released turn, or a tool result with no
matching section. Appending at the array end then placed the response's block after
that message, outside its own window, so the block never reached the viewport and
its very next delta failed the `find-block` assertion with "unknown transcript
block", ending the session (interactive `zig build run` reported it as an event
dispatch failure).

`append-block` now inserts at `block_start + block_count` and increments the
`block_start` of the responses that follow, so an interleaved message is ordered
after the response's blocks and every window stays its own. `inserted` clamps the
position to the array so a pre-existing inconsistency cannot build a sparse patch
sequence. The compaction path is the observed trigger: `complete-response`
publishes the ready status and then the completion, so an automatic compaction
starts while the queued prompt drains, and the summarizer survives long enough to
finish against a busy agent and report the discarded compaction into a streaming
turn.

Evidence: `tests/transcript-stream-interleave.fnl` (registered in
`tests/integration/standalone.zig`) drives real dispatch over the stock agent,
queue, compaction, and transcript fragments with session ordering: the ready status
and the completion, the queued turn's starting request, a first streamed block, the
discarded compaction's notice, then the turn's second block. It asserts that every
response's window holds exactly its own blocks, that the second block is created and
resolved, and that the notice follows the blocks it arrived during.
`tests/transcript-delta-state.fnl` keeps its generated creation/reset sequences and
scoped-lookup contracts unchanged.

### Web search tool and pluggable provider backends (2026-09-10)

`web_search` searches the web through one open `search-backends` registry, so the
agent sees a single tool schema while applications choose any provider.
`misa.search` owns vendor-neutral policy: argument validation, backend selection,
percent encoding, result and citation rendering, and the `misa.patch` overlay of
per-backend settings. Each `misa.search.*` backend owns one wire shape and one
completion normalizer. `misa.standard.tools.web-search` wires the tool, the
`tool.web-search/run` effect, the shared `tool/web-search-complete` handler, the
`search-backends` validator, and the Brave/Tavily API-key declarations.

Model providers are first-class search backends: `codex` reuses the ChatGPT
subscription credential and `openai` the OpenAI API key, both against their
existing Responses endpoints with the hosted `web_search` tool. `brave` and
`tavily` call dedicated search APIs through new native API-key providers, and
`searxng` queries a configured self-hosted instance. `config.tools.web_search`
selects the backend (default `codex`), bounds `max_results`, sets shared
`timeouts`, and carries per-backend overrides without changing the tool schema or
agent continuation. The native trust table binds the `brave` and `tavily` keys to
exactly their search origins, and both appear in `misa login`. A backend entry is
`{build, complete}`: `build` returns one native effect and `complete` renders its
completion, and the shared validator enforces the shape at install. The
provider-backed Responses backend additionally lets its own settings override
`model`, `url`, `instructions`, `tool_choice`, and `tools`, so a different
hosted search tool is configuration rather than a new adapter.

Evidence: `tests/web-search.fnl` (registered in `tests/integration/standalone.zig`)
covers settings defaults and malformed sections, shared argument validation,
percent encoding, result and citation rendering, every backend's request and
completion shape, per-backend overrides, failure text, backend validation,
backend selection, input immutability, and an installed application whose
`misa.catalog :search-backends` drives the effect and completion handler. Native
`src/auth/root.zig` tests cover the new trusted origins and API-key providers.
`tests/stock.fnl` and `tests/integration/configs/native-tools.fnl` install the
fragment, and the MCP listing assertion confirms `web_search` reaches the shared
tool registry. No live search service or provider request is part of the suite.

### Conversation compaction under a background model role (2026-09-10)

`misa.compaction` replaces a long canonical conversation with one summarizer
handoff when it approaches the selected model's context window, or on demand
through `/compact`. Summarization is an independent, tool-free provider request
resolved through `misa.models.for-role(db, "summarizer")`; the request carries a
bounded plain-text rendering of history and a handoff prompt, and the extension
owns no rendering. Only a summary produced for an unchanged conversation replaces
canonical history, installed as a user handoff plus a short assistant continuation
so the next request still alternates roles; empty, failed, unhelpful, or stale
summaries leave history untouched and report the reason. The visible transcript is
never rewritten, so scrollback and selection are preserved.

Automatic compaction starts from an `agent/status` ready event once the estimated
prompt reaches `threshold * (context_window - reserve_tokens)` with at least
`min_messages` messages, and a conversation that already triggered an attempt is not
retried until its scalar signature changes. The controller is pure: it patches
`db.compaction` (active request, monotonic sequence, last-attempt signature) and
emits ordered effects without mutating prior state. Role or catalogue changes
reconcile an active request through explicit `model/*` events rather than
registration order; draft submission is held through the editor lifecycle service
while a request runs. Usage is published as `compaction/usage` and consumed by
`misa.costs`.

Automatic compaction is gated on a resolvable summarizer model: an unconfigured
session queues nothing rather than repeating a refusal on every turn, matching the
tool-summary extension's documented behaviour. The trigger therefore fires only when
a model is assigned to the role _and_ the selected conversation model declares a
`context_window`, since the budget is unknown without one.

Evidence: `tests/compaction.fnl` (registered in `tests/integration/standalone.zig`)
covers settings validation, canonical history rendering and byte budgeting,
provider-reported versus heuristic token estimates, the automatic trigger and its
guards, explicit refusal paths, the tool-free request, streamed summary
accumulation and usage, applied/discarded/failed outcomes, cancellation,
reconciliation, reset sequencing, and the command/action/lifecycle declarations.
`tests/compaction-cascade.fnl` installs the fragment and drives real dispatch: it
pins both required conditions (an assigned summarizer and a declared context window),
asserts an unconfigured session stays silent and keeps history, follows the applied
handoff from trigger through the streamed summary to a two-message history, checks
the lifecycle query holds and releases draft submission, and covers `/compact` both
refusing and opening its dialog. `tests/extension-style.fnl` checks the module
surface, and `tests/stock.fnl` exposes `misa.compaction` as a stock fragment
alongside the catalog and settings entries. The installed fragment is part of
`tests/integration/configs/providers.fnl`, so application validation covers it.

### DeepSeek V4.1 Flash facts, wire contract, and peak pricing (2026-09-10)

DeepSeek replaced V4 Flash with V4.1 Flash (`deepseek-flash`) and retired
`deepseek-v4-flash-vision-exp`; both legacy names are still accepted, served, and
billed as V4.1 Flash. This adapts the review in
[oh-my-pi#11509](https://github.com/can1357/oh-my-pi/pull/11509) without adopting
its catalog format: the model list still comes from the provider, and the published
facts are declared separately.

`misa.providers.deepseek` declares facts for known identifiers only — context
window, off-peak prices, and the reasoning ladder — and the wiring merges them onto
discovered rows through the existing `models/update` mechanism, the same path
Claude uses for account-derived context windows. `deepseek-flash`,
`deepseek-v4-flash`, and `deepseek-v4-flash-vision-exp` share the V4.1 Flash facts;
`deepseek-v4-pro` keeps its own. An identifier outside that table stays catalogued
without invented limits or prices, so a future release appears in `/model` while its
facts remain explicitly unknown. No static model list is introduced, and a
declared fixed catalogue still supplies its own facts because enrichment runs only
after discovery.

Protocol-level changes are provider-requested rather than DeepSeek-specific.
`misa.protocols.openai` gained an optional `max_tokens_field` for the output-limit
field name and an optional `reasoning_content_field` that replays captured thinking
blocks onto that wire field; the DeepSeek transport selects `max_tokens` and
`reasoning_content`. DeepSeek requires reasoning replay when a conversation carries
tools and ignores it otherwise, so the adapter emits the field whenever thinking was
captured or the message carries tool calls. DeepSeek's own serializer fragment omits
`tool_choice` and `parallel_tool_calls` entirely, since forced tool choice is
rejected in thinking mode, and `reasoning_effort` is declared as `none`, `low`,
`high`, `max` for the whole V4 family, defaulting to `high`. One ladder covers both
SKUs because DeepSeek's API advertises it for both; third-party catalogues still list
Pro as `high`/`max`, the same stale curation
[oh-my-pi#8406](https://github.com/can1357/oh-my-pi/pull/8406) corrected for `low`.
`none` is DeepSeek's documented thinking-mode toggle. Reasoning turns emit
no stream bytes while the model thinks, so the transport defaults its first-byte and
idle budgets to five minutes; configured `timeouts` still override either value.

Cost accounting gained time-of-day prices. A rates record may declare
`peak = {multiplier, windows = [{start_hour, end_hour}], weekdays?}` with half-open
UTC hours and Lua weekday numbers (`1` is Sunday). `misa.costs.start-response`
resolves the multiplier against the request's wall-clock start and snapshots the
resulting rates, so retries, late completions, and interrupted responses cannot move
a turn between price tiers, and `interrupt-response` reuses that snapshot. Because
extension code cannot reach the OS library, `misa.time.utc-parts` exposes an
instant's UTC hour and weekday beside the existing `misa.time.local-datetime`; the
framework still owns all clock and locale access, and current time remains a
clock coeffect. Model previews state the peak windows beside the off-peak rates
instead of hiding the multiplier.

Evidence: `tests/deepseek-models.fnl` (registered in `tests/integration/standalone.zig`)
covers declared facts and the absent-facts case, alias pricing, the reasoning
ladders, serializer acceptance and rejection, both output-limit paths, the request
body, reasoning replay with and without the transport field, enrichment onto a
discovered catalogue through the real `models/update` handler, peak/off-peak/weekend
multipliers, and preview wording. `tests/integration/configs/providers.fnl` now
installs the DeepSeek fragment so application validation covers it.

### Event-scoped routing replaces global middleware (2026-09-07)

The framework no longer registers or executes global before/after interceptors.
`register/event-route` declares a source event type, subscription context, integer
priority, and pure resolver. Only routes for that source event run. Nil context
is inactive; false is a valid context. A winning resolver returns one semantic
event, handled in the same transaction without recursive routing. Equal winning
claims fail rather than depending on registration order. State/effects remain
ordinary handler results; input routes cannot modify transaction envelopes.

Dialog capture, picker palette access, picker capture, global actions, selection,
scrolling, history, and editing now have explicit precedence. Model, effort, and
omnipicker shortcuts use action declarations, with no parallel routing callback.
Alt decoding is recognized directly by keybinding lookup. Ambiguous bindings
within a context fail instead of silently selecting the first registration.
Modal interruption resets state in an explicit editing handler before queuing
normal editor input. Editor state initialization owns the insert-mode default;
Escape's cursor rule no longer depends on an input interceptor initializing it.

`tests/routing-state.fnl` exercises real dispatch in reversed route registration
orders, overlapping modal states, raw Alt/Escape, selection scrolling, multiline
history boundaries, unknown-key capture, source-event isolation, nonrecursive
routing, false/nil contexts, tie/invalid-result errors, rollback, and exactly-once
effects. Editing properties include missing-mode Escape, busy cancellation, and
EOF after modal reset. Test fixtures now observe ordinary handlers/effects or
declare scoped routes instead of requiring production middleware.

Verification passed: full ReleaseSafe baseline tests, installed build, installed
PTY interactions, and a 16-block/one-sample redraw/stream smoke check of the
migrated benchmark fixture (compatibility only, not new performance evidence).
Effort input profiles now explicitly load shared action routing. A final bounded
review found no remaining cutover blocker. Overall handoff reconciliation is
recorded above.

### Explicit command dispatch (2026-09-07)

Command producers now emit `commands/invoke`. Its owning handler normalizes
arguments and records preferences, then queues the declared execution event.
No command interceptor remains: execution events containing command metadata
are not reinterpreted. Choice availability is an explicit command predicate and
unavailable-event declaration; effort no longer intercepts command-choice events.
The focused test checks immutable inputs, correlation sharing, resumed choices,
unavailable choices, and real queued execution with exactly-once preference use.
The full ReleaseSafe baseline suite and installed build passed.

The routing audit's remaining input-hook findings are addressed by the
event-scoped cutover above.

### Live Kimi and final performance evidence (2026-09-07)

At `0c14a02`, a read-only native auth status check found the existing Kimi login.
The real provider usage request then completed with `unavailable=false` and two
normalized quota windows. Only login availability and window count were printed;
no credential contents or inference request were involved. The temporary probe
disabled model discovery and bounded request timeouts. This closes live Kimi
retrieval verification alongside the earlier native Codex and Claude checks and
the three-provider shared-dashboard regression.

The bounded final startup/rendering check is recorded in
`benchmarks/architecture-handoff-2026-09-07.md`: same-binary source versus installed
startup medians 588.329 / 44.032 ms; 300-block mixed redraw/stream medians 2.426 /
8.115 ms. Streaming maximum was 19.983 ms in ten measured samples. These are not
interactive startup, emulator paint, or universal 16 ms guarantees. Existing
correctness/recomputation tests remain the semantic oracle. No further benchmark
campaign or renderer complexity was added for this handoff.

### Shared usage flow and final presentation audit (2026-09-07)

`tests/usage-dashboard.fnl` feeds the real Claude, Codex OAuth, and Kimi usage
handlers into the shared dashboard and selected-provider fact query. It checks
all-provider refresh declarations, switching, exhausted/remaining values, Claude
extra-usage currency scaling, open-dialog refresh after a failed Codex request,
independence of other providers, and preserved old snapshots. It executes no
network/process effects and is not evidence of live Kimi account retrieval.
The focused dashboard contract and full ReleaseSafe baseline suite passed.

The final presentation audit's two semantic cutovers are now implemented:
response-cost metadata reaches transcript components as typed money facts,
timestamps remain raw milliseconds, and model pricing/context previews carry raw
facts into an open preview renderer. The shared `values` extension owns formatting
without a status dependency. Response metadata appears only on its designated
block; pending, estimated, partial unknown, and reported costs remain distinct.

`choices.preview` owns preview rendering and configurable type-to-renderer dispatch.
`choices.layout` measures its semantic lines once; picker rendering and input share
the resulting geometry. The geometry retains the raw preview model as well.
`tests/choice-preview.fnl` verifies compact/full pricing, zero rates, overrides,
narrow geometry, and action/link/animation metadata preservation. Models and costs
no longer create pricing summary strings. Explicit profiles and both extension
catalogs include the shared dependencies; no compatibility shim remains.

Verification passed: focused preview/cost/component/transcript regressions, full
ReleaseSafe baseline suite, installed build, and installed PTY interaction checks.
Traversal of transcript blocks and explicit collection reuse checks are not
themselves additional architecture defects.

### Domain initialization belongs to event handlers (2026-09-07)

Themes, component role selections, and animations now initialize in their owning
`app/start` handlers, together with persistence-load effects. They no longer
install global interceptors to recognize a single domain event. Existing state
is preserved and disabling persistence does not disable initialization.
`tests/presentation-startup.fnl` checks these contracts through direct handlers,
including immutable inputs and configured role maps.

Auth and model initialization now also use owning `app/start` handlers. Agent
startup queues `agent/startup` to observe settled auth state, and request options
queue `request-options/reconcile` after startup and model changes. Neither
consumer relies on which extension registered first. Auth readiness notifications
also check actual readiness; duplicate continuations cannot submit twice.
`tests/auth-state.fnl` exercises both registration orders, both discovery/status
completion orders, no providers, no auth extension, and no prompt.
`tests/model-startup.fnl` registers request options before models and checks
configured false values, switching, unavailable models, and restored availability.
`tests/initialization-state.fnl` now tests five direct owner handlers and rejects
initialization middleware.
Input routing is a separate ordered concern.

Queue/image lifecycle policies now contribute named subscription queries through
the setup-only `editor_lifecycle` service namespace. Contributors return boolean
`hold_exit` and `block_draft` facts, combined by `editor/lifecycle`; neither queue
nor images installs an interceptor or rewrites `agent/completed.keep_alive`.
The editor owns submission eligibility and queues its completion check after
other completion owners commit. Queue reservations clear through an explicit
submission-settled continuation rather than arbitrary completion notifications.

`tests/editor-lifecycle.fnl` covers both registration orders and terminal modes,
reserved/queued/rejected/auth-deferred submissions, interactive-only unsent draft
protection, independent third-party query contributions, preserved selection and
undo state, modal/command Enter during acquisition, and image completion without
automatic submission. These are real framework transactions with simulated
native effects; they do not alone prove native headless termination.
Review identified a rejection-ordering gap: releasing a reservation immediately
after `agent/submit` let an older exit check run before the rejection's queued
diagnostic/completion. Submission settlement now queues acknowledgement behind
those immediate events. The native `queued-rejection.fnl` fixture requires the
diagnostic and second completion before headless exit in both registration orders.
Verification passed: focused lifecycle/ownership regressions, direct native
headless runs in both orders, the full ReleaseSafe baseline suite (including
the native regression), installed build, and installed PTY interaction checks.

Verification: the focused startup/auth/model tests, full ReleaseSafe baseline
suite, installed build, and installed Ghostty PTY protocol/input suite passed.
The PTY check covers application output and input routing, not GUI painting.

Queue/attachment controls now use component resolution: `pending-prompt` receives
raw pending text and attachment count; `attachment-controls` receives pending
acquisition and attachment count. Both roles are overridable, and the default
components emit semantic styles and actions. This fixes their previously missing
hover backgrounds without changing resting styles. The focused
`tests/input-layer-components.fnl` checks all three action targets, hover leave,
immutable input/output, numeric counts, raw text, and role overrides.
Verification passed: focused component contracts, full ReleaseSafe baseline
suite, installed build, and installed PTY interaction checks.

The legacy `status.metrics` component and model have been removed. Minimal
profiles without the indicator registry use `status.indicators` too; component
swap coverage now exercises that role. Focused status/model tests check the
fallback model and reject installation of a parallel compatibility renderer.
The full ReleaseSafe baseline suite passed, including the component-swap fixture.
All bundled indicators now declare named queries with explicit subscription
inputs. `indicators/model` preserves typed facts, with only nil meaning omission;
zero and false remain visible. Models, effort, transcript detail, usage, activity,
and cost no longer supply value callbacks. The formatted total-cost projection is
removed in favor of a typed money fact. Provider-unavailable quota snapshots take
precedence over any numeric windows and invalid amounts remain unavailable.

The default component formats typed facts through an open `register/value-renderer`
dispatcher. Token compaction, ratios, percentages, currency, and activity frames
are presentation work. Animation selection uses its own named presentation query;
theme, hover, and animation changes do not invalidate domain fact dependencies.
The stale default configuration mapping for `status.metrics` is also removed.

`tests/indicator-values.fnl` checks late query registration, nil versus zero/false,
selective recomputation and shared fact identity, extensible formatting, invalid
facts/declarations, preserved shared spans, and separate animation presentation.
Owner tests cover quota unavailable precedence, typed money/options, and minimal
profiles. Full ReleaseSafe baseline suite, installed build, focused contracts,
and installed PTY interaction checks passed.

### On-demand Claude quota retrieval (2026-09-07)

The installed Claude Code 2.1.261 accepts the first-party SDK's experimental
`get_usage` control request with no inference prompt. The direct CLI probe exited
zero with a matching successful control response. Misa's actual native process
runner then retrieved and normalized the live account snapshot: `source=cli`,
`plan=max`, four quota windows and four extra-usage fields. No credential file was
read by Misa, and no model request was sent for either verification.

The provider now handles `usage/refresh` with correlated, coalesced, bounded
process requests. Failed/unsupported/malformed completions clear stale quota
values; stale request IDs cannot publish. The normalizer accepts zero/exhausted
percentages, leaves invalid/missing percentages unknown, preserves model-scoped
windows and extra-usage amounts, and only scales money with an explicit decimal
exponent. The new process's session totals are deliberately excluded. Stream
observations replace only their named window and retain other fetched windows.
The dashboard renders the additional semantic fields and respects unavailable
status even when reset-only windows exist.

`tests/claude-usage.fnl` covers transport declarations without launching a CLI,
coalescing/correlation, malformed records, unavailable/non-plan responses,
window validation, monetary scaling and stream merging. Claude stream/status
checks and the full ReleaseSafe/baseline suite pass. The external API remains
experimental; unsupported versions degrade to unavailable. See the linked
official SDK release and behavior documentation in README. This supersedes the
earlier on-demand Claude retrieval gap, not every provider/dashboard acceptance
item or billing feature.

### Structural-selection acceptance (2026-09-07)

The native PTY now verifies `v` plus `k` extends a range across two transcript
documents, and another `v` returns to single-node navigation. Focused selection
properties cover forward/backward and partial cross-document ranges, frozen
source during streaming, nested Markdown lists, CRLF/Unicode byte boundaries,
and source-only highlighting that excludes rails, titles and padding. The
open action registry is exercised with a non-clipboard consumer receiving the
same frozen ranges; copying is an optional action, not the selection model.
The focused suite and installed-catalog PTY pass. Annotation and conversation
forking workflows remain explicitly outside the current selection requirement.

### Default-theme and pointer acceptance (2026-09-07)

`tests/model-affordances.fnl` verifies the installed profile's value-only model
presentation, retained semantic label, displayed Alt-M binding, picker routing,
and corner-free default header/editor across modes, widths and Unicode input.
The native PTY now also opens the model picker by an SGR mouse click and Alt-M,
checks that both open the same picker, and cancels each through real input.
The installed-catalog run passes, including existing action and OSC-only link
hover/background/leave checks and actual all-motion reporting enablement.
`tests/component-resolution.fnl` verifies that leaving a target restores its
resting style without mutating shared semantic output. Action spans do not
implicitly acquire link styling; the default hover style supplies background
only. These close the theme/hover requirements. This is native protocol/output
verification, not a claim of testing an actual Ghostty GUI window.

### Interceptor responsibilities (2026-09-07)

Selected-provider quota refresh now consumes explicit model-selection/catalogue,
authentication and response-completion events instead of watching every
transaction. A queued `usage/check-selected` handler reads the completed model
state, so extension registration order cannot make it see the old selection.
Only provider changes and forced auth/completion checks schedule `usage/refresh`;
switching models within one provider does not. Provider disappearance clears the
remembered provider without inventing a request. Provider transports and their
in-flight coalescing are unchanged. Status tests exercise real dispatch with
status registered before the model owner, plus no refresh on streaming/redraw.

Immutable ownership does not by itself justify putting a domain transition in
an interceptor. Cost accounting's five-entry event switch has been removed from
the global `before` chain: its existing transition table now registers ordinary
patch-returning event handlers. Pricing snapshots, completion replay, interruption
and reset still belong to accounting; numeric and display subscriptions remain
separate. `tests/cost-state.fnl` checks handler purity and rejects an interceptor
registration, while the native costs fixture exercises actual event dispatch,
catalogue changes, reported costs, replay and reset.

Syntax's global transcript hook is removed. Canonical transcript mutations emit
scoped `transcript/updated` notifications after their existing effects. Syntax
consumes the notification and obtains current blocks through the transcript
owner's pure `transcript_blocks` service; it no longer infers append counts or
reads response storage indices. Highlight requests/completions stay effects and
events, while projections stay pure subscriptions. Tests cover scoped targets,
effect ordering, no-op mutations, missing/reset targets, duplicate notifications,
stale completions, and request coalescing. Bulk imports explicitly notify once
without a response target; the native fixture now follows that contract too.
The full ReleaseSafe/baseline suite and installed native interaction smoke pass;
single-sample fixture checks cover mixed streaming and Markdown redraw startup.
Those fixture checks are correctness checks, not a speedup measurement.
Input normalization/routing interceptors are a separate concern and are not
being removed indiscriminately.

Editing undo bookkeeping now receives the old editor, new editor and explicit
operation reason synchronously from editor transition handlers through a pure
service. The global after-hook and pre-input undo snapshots are removed. Modal
actions use the same policy, with undo/redo excluded from new history entries;
restore/steer clear history and structural editor selection in the owning
transition. Focused tests exercise direct handlers without interceptors, insert
groups, backspace-to-empty versus discard, redo invalidation, plain/noninteractive
policy, choice submission, and restored attachments/cursor. Startup initialization and
modal/input guards have separate ordering responsibilities and are not included
in this domain-hook removal.

### State and patch ownership audit (2026-09-07)

The remaining framework ownership gap was the transaction envelope returned by
`before` interceptors: handler application overwrote its `db` and appended to its
`fx`. Dispatch now owns a shallow envelope and effect-array copy before applying
handlers, preserving callback-retained snapshots and extension-owned envelope
fields. Application state and effect records remain shared; this does not
restore database drafts or reconciliation. The regression failed before the fix
and checks envelope preservation, effect order, extra fields, subsequent dispatch
and failed transactions.

A mutation-focused audit of all 63 bundled extensions found no remaining writes
to retained input state in their call paths. Candidate parser, component,
choice-row and provider-serializer writes target fresh construction; syntax
interceptors copy effect arrays before appending. This is an ordinary-table
ownership contract, not protection against third-party callback mutation.

Patch tests cover empty merges, explicit clearing/deletion/null, dense sequence
replacement, sparse/mixed patch rejection, invalid keys/data/controls, cycles,
metatables, exact nesting limits and repeated noncyclic references. Generated
properties compare an independent evaluator and verify retained snapshots,
sharing, idempotence, disjoint-update commutativity and unchanged patch input.
Dispatch tests cover patch-only results and failed transactions; native runtime
tests cover rejected view decoding and speculative subscription rollback.
These close the state/patch checklist items, not the separate presentation and
interaction audits. Earlier pending draft/envelope notes below are historical.
No performance claim is made for this correctness fix; the benchmark campaign
was stopped in favor of focused regressions and the normal suite.

### Mixed transcript and back-to-back input baseline (2026-09-07)

The native benchmark now has a `mixed` scenario: at 300 blocks it contains 150
responses, 75 completed tools with arguments/results, plans, and 75 code-fenced
replies. Measurement starts only after actual native highlighting returns
nonempty captures for every expected document. Ownership, retained identities,
stream bytes, visible code/tool output and OSC links are checked. Ten runs have
stable frame hashes; all six original Markdown hashes are unchanged.

The 300-block streaming baseline is 4.960 ms median of run medians. A separate
1,000-frame observation is 4.712 ms median / 19.681 ms maximum with a 25 ms idle
gap. Back-to-back input gives 17.019 ms median / 17.873 ms maximum, including the
driver's 16 ms presenter pacing; both runs have identical output and no missing
stream bytes. These measure input to complete PTY output, not terminal painting.
This is a closed-loop workload, not an independent-rate producer.
The every-frame target remains open, as do image/selection/streaming-tool and
populated-cost stress cases. See `benchmarks/mixed-transcript-2026-09-07.md` for
methods, limits and the next cost-lookup experiment lead. No production runtime
or rendering API was changed in this slice.

### Grapheme returns no longer allocate a closure (2026-09-07)

Compiled-code inspection identified a per-grapheme closure in the layout
primitive's final conditional return value. Binding that scalar first eliminates
the allocation without changing the compiler, width tables, segmentation or
public API. Exhaustive codepoint-width and generated mixed-text layout parity
passed against the saved baseline. An allocation probe shows that 64 ASCII width
calls dropped from 3648.203 to 0.203 KiB median allocated.

Ten alternating same-binary native comparisons improved all six workload
medians; 300-block streaming moved from 4.603 to 3.711 ms. A separate 1,000-frame
candidate observation measured 3.073 ms median, 13.703 ms p99 and 15.140 ms maximum,
with identical output to the baseline. The harness now reports empirical p95/p99.
This rested synthetic case does not close sustained or representative-workload
requirements. Methodology and limits are in
`benchmarks/grapheme-return-2026-09-07.md`.
Verification: complete ReleaseSafe baseline-CPU suite, installed build, compiled
Lua inspection and Unicode parity passed. PTY verification exposed and reproduced
a reader race accepting the tail of an older frame as the hover frame; the next
complete frame had the correct highlight. The test now requires both delimiters,
has partial-frame boundary checks, and passed 60 consecutive installed-catalog
runs. No hover implementation change was needed.

### Syntax deltas remove whole-catalog validation (2026-09-07)

Deeper profiling identified a larger cost than transcript block-vector updates:
the syntax interceptor replaced its complete state, including every parsed
document, after every event. Request construction now returns data; model updates
publish narrow document/pending-request patches. Completion similarly updates
only its pending request and document slots/revision. Unchanged input returns
the transaction itself. State validation and the public patch API are unchanged.

Ten same-binary alternating native comparisons with matching complete frame
hashes measured 300-block streaming at 7.839 → 4.474 ms (median of run medians),
and redraw at 4.206 → 3.245 ms. A separate 200-frame streaming observation was
4.299 ms median / 18.926 ms maximum; this does not close the every-frame budget
or representative-workload requirements. Unicode clipping and transcript
projection are now the dominant visible profile stacks. Small-workload results,
methodology and the rejected entry-merge API experiment are recorded in
`benchmarks/syntax-deltas-2026-09-07.md`.

Tests add 300-document request/coalescing/sharing checks and unchanged transaction
identity. The patch audit also corrected two tests that accidentally used table
keys instead of numeric keys (`1f39336`). No entry-merge API is retained.
Verification: focused syntax/state tests, the complete ReleaseSafe baseline-CPU
suite, installed build, and both installed-catalog and worktree-catalog PTY
interaction regressions passed.

### Packaged catalog verification (2026-09-07)

`nix-build -A default --no-out-link` passed build, check and fixup for the runtime
at `9ae1008`, producing
`/nix/store/zhczkbg9cpqs6qch7sn1i63d1pnxxrly-misa-0.1.0`.
The output contains 63 build-translated Lua extensions, and its installed default
configuration matches `config/default.json` byte-for-byte.

The PTY regression previously always forced worktree extensions. It now supports
`--installed`, which clears the source override and exercises the executable's
installed catalog. This mode passed against both `zig-out/bin/misa` and the Nix
store executable, including a run with an intentionally invalid inherited
`MISA_EXTENSION_DIR`. The regular worktree mode remains supported. The packaged
test covers action/link hover, images, editor/queue/history interactions, selection,
scrolling, RGB and OSC links. This closes the stale package-build verification
gap from the AOT startup changes; it is not a new startup performance measurement.

### Subscription-core requirement audit (2026-09-07)

Direct inspection of `src/lua_runtime/subscriptions.fnl`, framework dispatch/
commit/rollback, the native decoding-rejection test, and subscription test sources
establishes the core guarantees below. This closes the subscription-core checklist
item, not the separate migration of all presentation consumers or rendering costs.

| Requirement           | Implementation and evidence                                                                                                                                                                                           |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Explicit dependencies | Registration validates static query vectors; dynamic declarations are checked during evaluation. Tests cover constant/static/dynamic computations and invalid declarations.                                           |
| Safe keys             | Typed, length-delimited canonical encoding rejects nonfinite numbers, metatables, cycles and invalid vectors. Generated tests compare copied queries and distinguish extended queries.                                |
| Missing values        | Dependency vectors carry `n`; tests distinguish trailing nil from false and verify memoization identity.                                                                                                              |
| Cycles/depth/re-entry | Active-query and depth guards reject recursive graphs; tests cover changing-key deep chains and callback attempts to query/clear/close/fork the evaluating scope.                                                     |
| Bounded ownership     | Per-consumer caches evict to capacity, copy only their index on fork, and clear references on close. Capacity must now be a positive finite integer; public construction cannot inject inherited entries.             |
| Transaction isolation | Staged entries publish only after successful evaluation; framework forks commit with state. Lua-failure, explicit rollback, component-cache rollback and native-decoding rejection tests retain committed identities. |
| UI independence       | The core imports no renderer or framework globals; standalone scope tests use ordinary data. Explicit consumer tests query and close a separate scope without rendering.                                              |

The audit found and fixed two constructor boundary holes: false silently selected
the default capacity, and the internal inherited cache argument was exposed
through the public constructor. Infinity was already rejected by the modulo-based
integer check; a finite check is now explicit and regression-tested. Callback purity and
immutable inputs remain documented caller contracts, not a mutation sandbox.
Verification: focused scope/transaction tests, the complete ReleaseSafe
baseline-CPU suite (including native rejection), installed build, and PTY
interaction regression passed.

### Unpainted selections retain navigation targets (2026-09-07)

Removing selection color from chrome exposed a viewport dependency on painted
spans: whitespace-only selected documents could no longer reveal themselves.
Decoration now supplies an explicit `selection_anchor` on the first output line
when no visible span can be highlighted. The viewport recognizes that target
without requiring a background change, and still filters by the focused document
ID for cross-document ranges. Tests reproduce both the missing anchor and the
viewport failure, then verify that original output and title styling are unchanged.
Focused selection/viewport tests, the complete ReleaseSafe baseline-CPU suite,
installed build, and PTY interaction regression passed.

### Selection highlighting follows content provenance (2026-09-07)

Whole-document decoration incorrectly colored title chrome, rails and padding;
plain-text rendering lacked the source flags needed for partial range decoration.
Decoration now applies to source-marked content only. Plain-text output and
thematic rules carry source provenance; a fully covered source block can highlight
derived content (such as a rendered rule) without pretending its glyphs occur
literally in the original text. Unselected documents retain their original view
identity. Regression tests cover title/rail/padding exclusion, plain ranges,
unselected identity and selecting a rule within a larger document.
Focused selection tests, the complete ReleaseSafe baseline-CPU suite, installed
build, and PTY interaction regression passed.

### Frozen selection and live syntax source mismatch fixed (2026-09-07)

A regression test reproduced future stream text appearing inside a frozen
selection: transcript composition replaced the model's text with the selected
snapshot but retained live chunks, and syntax lookup preferred those chunks.
The component consequently received a live parsed document alongside frozen text.
Selected presentation models now discard their live chunk field after choosing
the frozen text. Canonical transcript state is unchanged; mismatching live syntax
is omitted and Markdown renders the selected source instead. Leaving selection
restores the live projection. The test checks frozen/live output and preservation
of canonical stream chunks. This does not claim that historical syntax captures
are retained by the selection snapshot.
Focused syntax/selection tests, the complete ReleaseSafe baseline-CPU suite,
installed build, and PTY interaction regression passed.

### Width-table lower-bound experiment rejected (2026-09-07)

A table-derived early exit from Unicode interval searches passed exhaustive
codepoint-width parity and 500 generated mixed-text cases, plus complete native
frame hashes across ten A/B pairs. Native timings did not establish a win:
300-block streaming median-of-run-medians changed from 7.242 to 7.550 ms and
other workloads were mixed. The production edit was reverted. The new
`benchmarks/layout-parity.fnl` probe is retained, with the exact rejected change
and results in `benchmarks/layout-range-bound-2026-09-07.md`. Clipping remains
an open performance target; no runtime speedup is claimed from this experiment.

### Syntax collection read once per transcript projection (2026-09-07)

The profiled repeated-query cost came from calling the syntax collection
subscription for every block. Collection access and pure model lookup are now
separate: `syntax_projections(db)` supplies an immutable snapshot and
`syntax_projection(snapshot, model)` checks the source without entering the
subscription engine. Transcript composition fetches once. No extra cache or
state-validation exemption is introduced.

Ten alternating native A/B pairs preserve complete frame hashes. At 300 blocks,
median-of-run-medians moves from 5.162 to 4.507 ms for redraw and 9.995 to 7.571 ms
for streaming. A second ten-pair small-workload check finds no consistent
one-block regression. Details and limitations are recorded in
`benchmarks/syntax-snapshot-2026-09-07.md`. Focused tests, the complete ReleaseSafe
baseline-CPU suite, installed build, and PTY regression passed. Full vector
replacement validation and Unicode clipping remain performance targets.

### Native transcript latency baseline (2026-09-07)

`benchmarks/native-transcript.py` now exercises keyboard-to-completed-frame latency
through the default native UI with 1/16/300 Markdown blocks. Every frame validates
sequence and output; redraws have a byte oracle, and streaming also checks exact
state and sharing. Back-to-back inputs encounter the driver's 16 ms frame pacing.
With an untimed 25 ms idle gap, 300-block streaming measures roughly 10 ms median
but 21–23 ms maxima across three sessions. The full-path latency requirement is
therefore still open. See `benchmarks/native-transcript-2026-09-07.md` for exact
scope, limitations and reproducible commands. Next: attribute native-path costs
and expand to tool-heavy/multi-response workloads; do not infer completion from
the faster Fennel-only measurements.

Steady-state profiling now excludes startup and supports 200-frame runs. A native
capture placed about 95% of sampled user CPU cycles in LuaJIT/generated code;
a separate Lua sampler identifies recursive patch materialization, Unicode
clipping, and repeated subscription queries as concrete next targets. The longer
uninstrumented run still has 24.380 ms maximum latency. No production optimization
has been claimed from these diagnostic changes.

### Claude stream quota observations (2026-09-07)

The existing open record dispatcher now handles `rate_limit_event`, normalizing
the CLI's fractional utilization, affected window, status, and reset timestamp
into provider-owned usage facts. A separate event reducer publishes those facts
and notifies the shared dashboard. Missing/invalid utilization clears prior
percentages; status alone does not invent a percentage. Account/session identifiers
are excluded. Tests cover generated batching invariance with quota records,
zero/exhausted/unknown values, malformed records, and replacement of stale facts.
This uses the official Python SDK parser/type definitions, not an inferred HTTP
schema. Only the latest affected window is represented; this is not the full
account snapshot or on-demand Claude quota retrieval still required above.
Focused Claude/status checks, the complete ReleaseSafe baseline-CPU suite,
installed build, and PTY interaction regression passed. No live Claude inference
request was made for verification; live stream acceptance remains unverified.

### Codex quota retrieval verified (2026-09-07)

The provider now handles targeted/broadcast usage refresh through the native
Codex credential and account metadata binding. A read-only live request to the
user-supplied `/backend-api/wham/usage` endpoint returned HTTP 200; a second probe
through the actual provider adapter produced three normalized windows and a plan
type. Probes redacted response strings and did not expose credentials. Tests use
synthetic values, checking percentage/duration/reset facts, nullable windows,
malformed values, coalescing, stale completions, failure clearing, and exclusion
of account identifiers. Native tests constrain the new credential trust to the
exact endpoint. The documented App Server interface is not treated as a schema
specification for this HTTP endpoint.

Verification: focused Codex usage/stream and status tests, the complete
ReleaseSafe baseline-CPU test suite, installed build, and Ghostty-input PTY
regression all passed. Dashboard tests verify numeric reset fields and the
selected-provider widget against the adapter's normalized output.

This supersedes earlier Codex-retrieval pending notes below. Claude retrieval,
credit/billing facts beyond quota windows, and final dashboard acceptance remain
open; reset delays are snapshots, not live countdowns.

The initial implementation cloned every transaction and reconciled the
entire database. Its subscription cache used mutable table identity, unbounded
global ownership, delimiter-based query keys, and array lengths for nullable
dependencies. Passing the old suite does not validate those contracts.
Views now receive shared state, so rollback and mutation boundaries need explicit
verification before declaring the state migration complete.

The framework now enforces patch-only handler results and no longer clones or
reconciles the database. Remaining mutation-oriented integration fixtures were
converted, including Lua syntax completions and editor/queue setup. Direct
dispatch tests assert identity at coeffect, handler, and view boundaries; no-op
root identity; retained-state preservation; and rollback on invalid or legacy
results. Ordinary input tables are immutable by contract, not write-protected.
Interceptor transaction-envelope mutation and projection ownership still need
the final audit; this cutover does not complete rendering or performance work.

## Verified migration slices

The default header no longer has ornamental corner prefixes, and the editor's
insert/continuation prompt uses a straight rail instead of opening/closing corners.
Normal/visual mode markers remain distinct; their glyphs are declared as data.
Focused tests cover multiline input, narrow prompt widths, Unicode, and cursor
byte bounds. The PTY checks the initial default frame for the removed corners.
Structural borders in Markdown tables/code and dialogs are unchanged.

The Ghostty-protocol PTY fixture now includes a visible Markdown footer link.
Native SGR motion must set `hover_link` rather than `hover_action`, emit the hover
background, and remove it on exit while preserving OSC hyperlink sequences in
both frames. Together with the model-button test, this covers action and OSC-only
link hover through decoding, hit testing, state, and presentation after the render
cutover. This remains a PTY protocol test, not a live Ghostty GUI acceptance check.

Transcript rendering now uses a collection-owned component projection. Semantic
output, incremental Markdown hints, and themed views survive unchanged items;
hover/theme changes do not rerun semantic component callbacks. The global message
document cache and reset handlers are removed. Syntax and cost enrichment use
incremental collections too, avoiding the earlier flat-cache eviction failure.
Tests cover 300 enriched items, retained view identity, changed/removed items,
layout/theme/component changes, hover precedence, and speculative rollback.
The interleaved 300-block probe improves average redraw CPU cost from about
28.7 to 2.6 ms/frame, and combined streaming work from 25.1 to 1.8 ms/frame.
See `benchmarks/component-collections-2026-09-07.md` for the single-block overhead,
exact measurement scope, and remaining native/mixed-transcript verification.
Model preparation still traverses all blocks; this is not the entire render audit.

Replacement materialization now allocates only when values differ or keys are
removed, while retaining full recursive validation. Equal-identity invalid-data
tests prevent sharing from becoming a validation bypass. Isolated A/B probes show
repeatable replacement CPU/allocation improvements, but whole-transcript timings
do not establish a speedup; the dated patch-replacement report records both.
The transcript probe now supports a saved baseline state implementation and
combined update/render sample totals to avoid misleading GC phase attribution.

`benchmarks/transcript-render.fnl` establishes a whole-transcript baseline beyond
the earlier individual-component probe: real model preparation, themed component
rendering, and transcript delta/patch work are timed separately across 1, 16, and
300 blocks. Complete frame records remain identical across repeated runs, and
streaming preserves old state and unrelated block identity. The recorded baseline
exposes nontrivial update costs at 300 blocks; replacement materialization and GC
need attribution before changing cache ownership. Native presentation and mixed
transcripts remain unmeasured; see the accompanying dated report for scope.

Fixed subscription dependencies are now ordinary query-vector data, validated
at registration. Query-dependent inputs retain the function form; bundled model
and aggregate cost projections no longer allocate their fixed declarations in
callbacks. Tests cover static nullable inputs and transactional rollback, mixed
static/dynamic cycles, invalid declarations, empty constant dependencies, and
argument-dependent queries. This simplifies subscription expression without
claiming to resolve the message component's global projection-cache ownership.

The Ghostty-protocol PTY test now sends SGR mouse motion over the default model
button and away again. It checks the native hover action, emitted hover RGB
background, absence of that background at rest, and removal on exit. Inspection
completion is not a presentation barrier, so the test also awaits the emitted
frame. This verifies decoder-to-state-to-render integration, not a live Ghostty
GUI or end-to-end OSC-only link hover; those acceptance checks remain open.

Automatic Kimi refreshes now coalesce while a request is in flight, retaining one
follow-up refresh rather than launching overlapping operations. Success and
failure both release the in-flight slot and honor the queued refresh. Tests
verify request IDs, late completions, repeated-trigger identity, failure retry,
and removal of prior quota values after a failed refresh.

Quota refresh now follows selected-provider changes, authentication readiness,
and response completion/interruption, with provider-addressed events. Kimi ignores
requests for other providers; manual dashboard refresh still broadcasts. Immutable
policy tests cover provider transitions and ensure streamed token events cause no
requests. Codex investigation found the documented App Server
[`account/rateLimits/read`](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt)
route, but did not establish a direct HTTP contract for
Misa's native OAuth transport. No CLI-account substitution or guessed endpoint
was added; that integration remains pending.

The default status layout includes a provider-neutral `plan` indicator consuming
normalized windows for the selected model's explicit provider field. It reports
the tightest known remaining percentage, distinguishes exhausted/unknown data,
and opens the usage dashboard through the action registry. It consumes the last
fetched data; automatic refresh and other provider fetchers remain pending. The
actual default config now also requests model value plus hotkey, matching the
fallback layout; tests check that profile rather than only the fallback.

Kimi coding-plan retrieval now handles the open `usage/refresh` event with a
credential-referenced regional `/usages` HTTP request. Completion normalizes quota
windows, rejects superseded responses, and dispatches `usage/updated`; the shared
dashboard refreshes its semantic sections only while open. Tests cover request
descriptors, region selection, missing/malformed data, numeric quotas, stale
results, and closed-dialog behavior. No live account was queried. Claude/Codex
retrieval, selected-provider widgets, and authenticated acceptance remain pending.

Structural visual ranges now retain a document-position anchor across depth and
document navigation. `selection_ranges` exposes ordered frozen source slices;
copy joins those slices while per-document projection drives transcript freezing
and decoration. Selected lines retain document IDs so the viewport reveals focus
rather than an earlier highlighted document. Tests cover forward/backward ranges,
partial endpoints, CRLF/Unicode copying, live-source changes, decoration, and
focus-following. Live-terminal acceptance and broader selection affordances remain
separate verification work.

The bundled extension catalog now declares ID/path pairs once as data. Discovery
IDs derive from those entries, runtime resolution searches them, and build-time
translation consumes their paths directly. Tests retain exact public mappings
and reject duplicate IDs/paths and unsafe relative paths. Adding a bundled
extension no longer requires synchronizing a list with an imperative lookup
chain. This is catalog-data cleanup, not the pending transcript-cache cutover.

The per-message subscription-cache prototype was rejected after a working-set
probe: three redraws of 300 messages rose from 34.399 ms median to 1075.684 ms,
with identical semantic output. Per-message entries exceeded the shared bounded
scope and repeatedly evicted one another. The prototype and its prior-result
compute hint are reverted; `benchmarks/message-cache-2026-09-07.md` and the
archived patch preserve evidence. The next cutover must own a structurally shared
transcript projection collection rather than compete for one cache slot per
historical message. Message-cache replacement remains unfinished.

Incremental Markdown layout is now explicit immutable data:
`markdown_view.project(text, options, previous?)`. It returns reusable block
entries and lines without mutating any prior projection, replacing the mutable
view/parser closure and weak-key layout cache. Message wrappers retain immutable
entries rather than mutating a view object. Tests cover branches, captures,
unchanged identity, streaming/full-layout parity, and large documents. The
message-level cache still needs subscription-owned lifetime and rejection
handling. `benchmarks/markdown-layout-2026-09-07.md` records a focused parity and
timing probe; this is not end-to-end selective rendering evidence.

Component render inputs now use the same immutable-by-contract ownership as
state and subscriptions. The boundary no longer deep-copies model/context data,
and syntax projections cross as ordinary tables instead of identity-preserving
callback wrappers. Tests assert boundary identity, nested syntax sharing, default
presentation input preservation, and unchanged cached output after theme/hover
resolution. Custom components must allocate their own changes rather than mutate
inputs; this is an intentional contract cutover, not a compatibility mode.
Render-owned cache removal and end-to-end rendering measurements remain pending.

Markdown layout now compares actual parsed-document and capture identities,
not syntax revision bookkeeping. A regression reproduced wrong highlighting when
independent snapshots shared text and revision but had different captures. Tests
cover switching back to the retained snapshot, replacing the explicit document,
unchanged-input reuse, and revision-only changes. This corrects the existing
cache's dependency contract; it does not remove the render-owned document cache
or establish an end-to-end rendering speedup.

Cost projections now use explicit subscription dependencies: collection-level
numeric totals, a dependent formatted projection, and per-response projections.
Tests verify unrelated-hover reuse, untouched-response identity, discarded scope
forks, empty/reset state, and retained results. Composed generators check response
sequences against an independent accumulator. This establishes ownership and
invalidation contracts, not a measured rendering speedup. The mutable document
cache in `component/message.fnl` and unified render composition remain unfinished.

The bundled runtime core now joins installed extensions in build-time Fennel
translation. Core Lua source is embedded through declared generated-file inputs;
the compiler remains available for custom modules. Release/baseline tests cover
bootstrap stack cleanup, core source locations, and custom Fennel failures.
Interleaved native startup-through-EOF measurement fell from 81.000 ms median to
33.557 ms (ten runs per binary, identical output); see
`benchmarks/core-startup-2026-09-07.md`. Interactive readiness and rendering/streaming
costs still need their own evidence.

Editing before/after policies, startup initializers (themes, components,
animations, auth, models), and cost accounting now return new transaction
envelopes. Editing properties check the whole input transaction and retain the
after-policy result. Initializer tests assert input preservation, unchanged
branch identity, idempotence, and no-op handling of unrelated events; cost tests
also check envelope ownership. Command and syntax policies remain to audit.

Command normalization and syntax scheduling now return new transactions and
fresh effect collections. Direct command tests cover canonical arguments,
choice handoff/resume, one preference update per invocation/argument, persistence
payloads, no-op events, and input ownership. Syntax properties check the full
before/after transaction. Syntax's external parser/result cache remains a
separate projection-ownership audit; this change does not make that cache a
subscription. Remaining event-routing interceptors still replace tx.event in
place and need the same envelope contract.

Syntax projection ownership is now cut over: a regression first demonstrated
that later resets erased the projection of a retained state. Parsed documents
and accepted capture arrays now belong to immutable syntax state; incremental
parsing takes the previous document explicitly. The external document/result
cache, pruning, and epoch protocol are removed. `syntax/projection` declares one
document-entry dependency and memoizes its projection through the subscription
engine. Tests cover retained-state rendering after subsequent updates/reset,
unrelated-state projection reuse, and existing rejected-completion/reset paths.
This is a correctness change, not a measured speedup: the cost of carrying parsed
data through patch validation still needs representative streaming measurements.

Model selection/status projections now use explicit `models/selected` and
`models/projection` subscription dependencies, rather than rebuilding from the
whole database. Affordance tests cover unrelated-state reuse, default changes,
missing selection, and retained-state reads. Model picker previews also copy cost
detail lines before adding context metadata, preventing repeated completions
from modifying shared provider cost data. This establishes dependency/ownership
contracts; end-to-end rendering performance remains unmeasured.

Usage dashboard presentation now supplies generic dialog sections/fields with
typed values. Dialog lifecycle carries and replaces those sections; the dialog
component renders them. Provider rows are sorted, deduplicated, and explicitly
unavailable when quota facts are absent. Tests cover raw values, precedence,
section updates, retained state, and visible output. Actual provider coding-plan
retrieval remains unfinished; semantic presentation is not quota integration.

Structural selection now groups Markdown lists, nests indented items, and keeps
continuation lines under their owning item. Source ranges include nested content
without consuming following paragraphs. Tests exercise bounds, Unicode/CRLF,
and visual sibling ranges copied through the real selection actions. Cross-
document ranges and broader transcript interaction remain unfinished.

Hover auditing found OSC-only links absent from the native hit map. Hit targets
now distinguish links from actions; pointer motion reports either, while clicks
still dispatch only registered actions. Component resolution applies hover
backgrounds to link-only spans and restores their resting style on departure.
Tests cover cloned maps, grapheme cells/clipping, action precedence, driver input
translation, and semantic-style preservation. Live-terminal acceptance remains
part of the final interaction audit.

Fresh startup attribution is recorded in `benchmarks/startup-2026-09-07.md`.
The isolated default-profile probe repeats deterministic setup/first-dispatch
semantics without executing native effects. Ten samples put Fennel compilation
at 527 ms median of 546 ms CPU total, making bundled build-time compilation the
next discriminating experiment. This is not full interactive startup latency,
and no startup optimization has landed yet.

The build-time compilation experiment now has 10 interleaved samples per mode
with identical startup output: probe median wall time was 565 ms from source and
21 ms from freshly generated Lua. See
`benchmarks/startup-precompiled-2026-09-07.md` for raw results and limits.
`tools/compile-fennel.lua` emits portable Lua sources, not bytecode. Installed
loading/build integration, custom-source diagnostics, and actual binary latency
remain required; this experiment does not change production startup.

Installed catalog extensions now use build-generated Lua with tracked source,
compiler, and generator inputs. Explicit source-directory overrides and literal
Fennel paths remain dynamic. The installed binary's default-profile EOF workload
improved from 582 ms to 82 ms median in ten interleaved samples per mode, using
the same ReleaseSafe/baseline executable and isolated accounts. See
`benchmarks/installed-startup-2026-09-07.md`. Runtime-core compilation, full Nix
package verification, and interactive startup acceptance remain outstanding.

The full Nix package build/check phase now passes with generated extensions.
Packaging verification also exposed local build outputs entering `lib.cleanSource`:
the unfiltered archive contained 3.5 GB of Zig cache plus zig-out/direnv content.
An explicit source filter excludes those directories without deleting local
files. The inspected filtered archive is 1.9 MB, evaluation assertions cover
exclusions and required compiler/extension inputs, and rebuilding that filtered
source passed the full package checks. Runtime-core compilation and interactive
startup acceptance remain outstanding.

The remaining bundled event-routing interceptors (actions, dialogs, keybinding
normalization, picker, command palette, and history) now return replacement events
without mutating the transaction. `tests/routing-state.fnl` covers full-input
ownership with composed generators and explicit modal precedence, history,
palette, and custom-action routing. No direct tx-field assignments remain in
bundled extensions; alias-based mutation and external projection caches still
require audit before marking the full state requirement complete.

Choice-session services now return immutable sessions, including narrowing and
view replacement. Input dispatch is extensible through `register/choice-input`;
rows and layout no longer mutate their input. The picker lifecycle returns
patches. `tests/choice-state.fnl` covers generated transition sequences, no-op
identity, rendering ownership, custom-view rotation, nested pickers, and false
values. Inline callers retain the returned session.

Editor handlers now return patches, with text edits dispatched through an open
`register/editor-edit` registry. `tests/editor-state.fnl` checks generated input
sequences, state/event ownership, Unicode cursor boundaries, attachment handoff,
command completion, busy cancellation, and projection ownership. The separate
Vim policy now applies patches too, with open motion/action registries.
`tests/editing-state.fnl` checks generated modal transitions, undo/redo bounds,
insert groups, discarded drafts, range operations, Unicode motions, and custom
transitions. Other state owners and provider reducers still need migration
before transaction drafts can be removed.

Dialog lifecycle/input and authentication handlers now return patches.
`tests/dialog-state.fnl` covers generated input sequences, stale correlations,
empty replacements, and the protected-input boundary. `tests/auth-state.fnl`
covers late provider registration, provider-keyed startup work, duplicate and
unrelated completions, and auth-owned dialog cleanup. Registry lookups live with
provider registration instead of taking setup-time snapshots.

Syntax request bookkeeping and completion handling now return persistent state;
moving an unchanged code block no longer mutates an older slot's source offset.
`tests/syntax-state.fnl` checks ownership, offset changes, reset/stale completion
handling, and generated single-slot request coalescing. The existing syntax
integration test still verifies rollback and external capture-cache ownership.

The fake and Codex providers now return persistent stream-state patches.
`tests/fake-provider-state.fnl` checks fixture ownership and replay;
`tests/codex-stream-state.fnl` checks state/record ownership, generated batching
invariance, terminal gating, reasoning summaries, and tool-call deltas. Codex
record dispatch is open through `register/codex-record`. The other provider
adapters and the agent/transcript owners still need migration.

The OpenAI-compatible protocol now uses persistent terminal-state updates and
an ordered `register/openai-delta` projection registry. Its generated tests
check batching invariance and record ownership; focused tests cover tool-call
fragments, stream isolation, terminal gating, transport errors, and explicitly
disabled discovery credentials.

Anthropic-compatible model discovery now accumulates pages with persistent
updates, sorts only owned results, and explicitly clears completed/failed state.
`tests/anthropic-discovery-state.fnl` checks generated page-boundary invariance,
input ownership, provider isolation, cursors, and empty authoritative results.
Anthropic-compatible stream records now use persistent state and open record,
block-start, and block-delta registries. Generated batching tests and focused
checks cover signed/redacted thinking, tool fragments, usage, terminal gating,
stream isolation, and credential-profile mismatch reporting. The Claude CLI
adapter is covered below; the agent/transcript owners remain to be migrated.

The Claude CLI adapter now uses persistent stream state and open record/event
registries. `tests/claude-stream-state.fnl` covers generated batching invariance,
input ownership, streamed-first/final-first tool identity, repeated tool results,
per-message text fallback, metadata-only blocks, terminal gating, and extensions.
The runtime regression fixture repeats final tool records and verifies exactly
two transcript tool starts for two distinct calls, provider-owned tool results,
and successful request completion.

Selection navigation now returns immutable state through an open
`register/selection-action` registry. Anchors use structural depth rather than
mutable frame identity; changing documents resets document-local ranges.
`tests/selection-state.fnl` covers generated empty/nonempty navigation, visual
extension, parent transitions, copy effects, custom actions, and non-mutating
rich-line decoration. Cross-document ranges and the broader transcript/render
cutover still need work; this does not complete the interaction requirement.

Status fact handlers now return immutable patches, with optional usage fields
replaced explicitly (including empty snapshots). Command registration no longer
depends on whether indicators are installed. `tests/status-state.fnl` checks
generated updates, unchanged-branch identity, empty replacement, event ownership,
and profiles with/without indicators. The existing usage dialog still needs the
semantic dashboard and actual provider quota retrieval listed above.

Agent stream start/delta/usage/provider-state/tool-result handlers now return
persistent patches. The open `register/agent-delta` registry separates delta
assembly from shared request correlation. `tests/agent-stream-state.fnl` checks
generated block assembly, ownership, tool identity, argument replacement,
nullable usage fields, opaque provider data, stale/cancelled inputs, and custom
delta reducers. Agent request/completion/tool orchestration and transcript state
still require migration; whole-transaction drafts have not yet been removed.

Agent startup/auth handoff, cancellation intent, reset, failure, and interrupted
stream finalization now return patches. Lifecycle tests check ownership,
generated cancellation/reset/error sequences, sorted tool cancellation effects,
partial text preservation, and queued startup cleanup on reset. Normal request
submission, response completion, and tool-result draining remain mutable.

The remaining agent submission/completion/tool-result paths now return patches
too. Requests receive completed history explicitly; normalization leaves provider
records intact, and canonical history strips transport-only metadata. Tests cover
both tool completion orders, duplicates, cancellation draining, unknown and
provider-owned tools, usage accumulation, and blocked submissions/continuations.
The agent no longer returns legacy `db` results or mutates input state. Transcript
state and framework transaction/subscription contracts still need their cutover.

Transcript deltas now use pure block patches through `register/transcript-delta`.
Full argument replacements no longer rewrite incoming events. Generated tests
cover state/event ownership, Unicode text byte accounting, preview truncation,
replacement after truncation, structured redaction, and custom delta reducers.
Transcript lifecycle helpers and the viewport's mutable projection cache remain
to migrate; the delta slice does not establish render-cache rollback safety.

Transcript block/response completion and interruption now produce immutable
blocks and response metadata. Tests cover generated fragment finalization,
timing/token rates, metadata attachment, argument compaction, and clearing stale
tool descriptions. Creation/standalone transcript handlers and viewport state
remain to migrate.

Transcript initialization/reset/detail/scroll handlers now return application
patches, and global input routing preserves its incoming transaction. Generated
control sequences and direct wheel/page-key checks cover ownership, retained ID
sequences, and clearing scroll anchors. Viewport projection state still lives in
a mutable closure and must be replaced before claiming rollback-safe rendering.
The redundant `messages.transcript` mirror has been removed; `messages.blocks`
is the sole block collection. Integration fixtures now inspect that canonical
collection instead of requiring two copies to remain synchronized.

Transcript creation/standalone handlers now retain immutable append results.
Generated creation/reset sequences validate response indexes, block ownership,
and input preservation; focused checks cover streaming creation, tool-result
attachment, and interrupted fallback messages. Generated legacy response IDs now
advance the counter. No transcript event handler returns a legacy `db` result;
viewport closure ownership and renderer-input mutation still require an audit.

Transcript presentation now uses an open `register/transcript-presentation`
registry with role/model results instead of closed kind branches. Selection
decoration no longer assigns into a component's returned line collection. Tests
cover projection/context ownership, cached component output, collapsed/expanded
thinking, selected tool results, and extension-defined presentation. This remains
separate from the pending unified render composition and subscription cutover.

The viewport closure has been removed. `transcript_viewport` derives immutable
window data; `ui_regions` exposes the root's ordered regions, including transcript
measurements. Scroll events derive current geometry and store only explicit
anchors/selection coordinates. Tests check speculative-render isolation,
snapshot rollback, selection reveal/manual scroll, streaming anchors, bounds,
and one transcript projection per viewport or scroll query. This is an ownership
cutover, not a measured speed improvement: scroll queries now derive layout, and
sharing that work with rendering remains part of subscription/performance work.

Effect-only effort controls no longer return application state; their input
policy returns a new transaction. Queue/image keep-alive policies no longer
modify incoming events in place. `tests/control-ownership.fnl` checks original
event/transaction preservation, pending-image input replacement, effort cycling,
selection, and unsupported options. Remaining model affordances and the framework
cutover are still pending.

Model controls now expose Alt-M and route it through the shared command picker
without mutating input. Default model status uses value plus hotkey while keeping
its semantic label. Indicator width comes from actual rendered spans, so hidden
labels no longer consume space and icon representations retain their icon.
`tests/model-affordances.fnl` covers those contracts and shared keybinding-span
ownership. Premature model subscription wiring was removed pending the framework
subscription contract; the selected-model projection remains pure.

`src/lua_runtime/subscriptions.fnl` now provides the replacement evaluator as a
UI-independent registry with bounded, disposable consumer scopes and speculative
forks. Direct tests cover typed canonical keys, nullable dependency counts,
identity reuse, dependency cycles, failed-query isolation, bounds, and disposal.
It is not yet wired into framework transactions: replacing the provisional
framework cache and connecting native commit/rejection ownership remain required.

The replacement evaluator is now embedded and wired into framework registration,
`misa.sub`, and explicit consumer scopes. Each dispatch forks the committed scope;
only `_commit` publishes it. `_rollback` discards pending state and memoization,
and native decoding/session-validation failures invoke that rejection path.
Direct transaction tests and a native decoding-rejection test verify memoized
identity survives rejection. The old draft/reconciliation mechanism still needs
removal; subscription reuse during that interim is not a performance claim.

Mutation-dependent terminal/worker fixtures are being migrated before enforcing
patch-only dispatch. Settled-frame, threaded-input, animation-clock, syntax-worker,
dispatch-limit, timer, and reset fixtures now declare patches; effect-only
transcript/picker/dialog fixtures no longer return state. The remaining fixture
migrations and actual removal of clone/reconcile are still required.

The shared test generators now include bounded integers and booleans, with
validated runner budgets to prevent accidentally vacuous test runs. Patch
properties compose generated operation sequences and check all retained snapshots
after subsequent updates, alongside the independent reference evaluator and
structural-sharing assertions. Generator contracts cover deterministic replay,
shrinking, bounds, and rejected invalid options; `tests/GENERATORS.md` documents
the deliberately small API and its shrinking limits.

The stock application is now assembled one directory at a time. A directory's
`init.fnl` is the module for that directory, and it merges the directory's own
declarations from the sibling `core.fnl`, when it has one, with its children. It
names only modules under its own directory, so `misa.standard` requires its
immediate children instead of enumerating 46 modules across four levels, and
`misa.standard.editor` is the stock editor while `misa.standard.editor.core`
stays the editor's own declarations. `misa.merge-definitions` merges catalog maps
while preserving function values and rejecting duplicate identities; composition
order stays irrelevant because installation still sorts by priority and ID. The
installed application is identical before and after: 44 catalogs and 923 entries,
and `tests/stock.fnl` points its eleven aggregate keys at the `core` modules, so
every existing fixture installs exactly the modules it did before.
`tests/extension-composition.fnl` enforces reachability from `misa.standard` and
per-directory locality, and names the three deliberately unstocked provider
modules (`command`, `fake`, `generic`) so omissions stay explicit.
