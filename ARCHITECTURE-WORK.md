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
