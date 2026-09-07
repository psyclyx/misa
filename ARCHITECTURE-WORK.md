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
- [ ] Claude: tool calls appear once across streamed and final records; regression
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
Vim editing policy still mutates editor and undo state; it must be migrated
before transaction drafts can be removed.
