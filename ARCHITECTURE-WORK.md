# Architecture and interaction work

This tracks the accepted scope while the implementation is in progress.
Completion requires behavioral evidence, not just passing existing tests.

- [ ] State: migrate bundled reducers, services, and interceptors to immutable
  updates; remove the transaction draft and whole-database reconciliation.
- [ ] Patches: settle empty-table semantics, validate data and controls, preserve
  sharing and rollback, and cover nested replacement/deletion and collections.
- [ ] Subscriptions: explicit query dependencies, safe query keys, nil inputs,
  cycle errors, bounded cache ownership, rollback, and UI-independent consumers.
- [ ] Presentation: semantic render data, open dispatch registries, unified
  renderable composition, and subscriptions replacing ad-hoc projections.
- [ ] Rendering performance: verify selective recomputation and streaming costs
  with representative transcripts, preserving interaction metadata through layout.
- [ ] Startup: profile and improve default startup; record reproducible measurements.
- [ ] Default theme: remove unwanted angled decoration; model value and picker
  binding must be distinct from semantic labels and correctly presented.
- [ ] Hover: pointer motion highlights clickable backgrounds only while hovered,
  with unchanged resting appearance and working click routing.
- [ ] Transcript interaction: structural navigation, Vim-style visual ranges,
  Markdown list structure, and a selection model independent of copying.
- [x] Claude: tool calls appear once across streamed and final records; regression
  fixtures cover provider-owned tools and completion boundaries.
- [ ] Usage: actual Claude, Codex OAuth, and Kimi coding-plan usage retrieval,
  a common dashboard, selected-provider indicators, and graceful unavailable states.
- [ ] Verification and handoff: focused commits, complete relevant tests, accurate
  documentation and measured performance claims.

Annotations and conversation forking were identified as future consumers; their
product workflows require later data-model decisions. The interaction architecture
must permit them without coupling structural selection to clipboard behavior.

## Audit findings to resolve

The initial implementation still clones every transaction and reconciles the
entire database. Its subscription cache uses mutable table identity, unbounded
global ownership, delimiter-based query keys, and array lengths for nullable
dependencies. Passing the old suite does not validate those contracts.
Views now receive shared state, so rollback and mutation boundaries need explicit
verification before declaring the state migration complete.

## Verified migration slices

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
