# Architecture and interaction work

This tracks the accepted scope and its verification evidence: the checklist,
the handoff summary, and a pointer to the dated audit notes, which live in
`docs/history/` because later entries supersede earlier ones.

- [x] State: migrate bundled reducers, services, and interceptors to immutable
      updates; remove the transaction draft and whole-database reconciliation.
- [x] Patches: settle empty-table semantics, validate data and controls, preserve
      sharing and rollback, and cover nested replacement/deletion and collections.
- [x] Subscriptions: explicit query dependencies, safe query keys, nil inputs,
      cycle errors, bounded cache ownership, rollback, and UI-independent consumers.
- [x] Dispatch cutover: remove the global interceptor chain; command policy uses
      explicit handlers and input routing uses an extensible, scoped dispatcher with
      verified modal precedence rather than extension registration order.
- [x] Presentation: semantic render data, open dispatch registries, unified
      renderable composition, and subscriptions replacing ad-hoc projections.
- [x] Rendering performance: verify selective recomputation and streaming costs
      with representative transcripts, preserving interaction metadata through layout.
- [x] Startup: profile and improve default startup; record reproducible measurements.
- [x] Default theme: remove unwanted angled decoration; model value and picker
      binding must be distinct from semantic labels and correctly presented.
- [x] Hover: pointer motion highlights clickable backgrounds only while hovered,
      with unchanged resting appearance and working click routing.
- [x] Transcript interaction: structural navigation, Vim-style visual ranges,
      Markdown list structure, and a selection model independent of copying.
- [x] Claude: tool calls appear once across streamed and final records; regression
      fixtures cover provider-owned tools and completion boundaries.
- [x] Usage: actual Claude, Codex OAuth, and Kimi coding-plan usage retrieval,
      a common dashboard, selected-provider indicators, and graceful unavailable states.
- [x] Verification and handoff: focused commits, complete relevant tests, accurate
      documentation and measured performance claims.

Annotations and conversation forking were identified as future consumers; their
product workflows require later data-model decisions. The interaction architecture
must permit them without coupling structural selection to clipboard behavior.

## Handoff evidence

The final production cutover is `93c6da0`. The current source and executable
checks map to the accepted requirements as follows:

Final gates passed:

- `zig build test -Doptimize=ReleaseSafe -Dcpu=baseline
"-Dtree-sitter-dir=$MISA_TREE_SITTER_DIR" --summary all`: 250/250 tests,
  101/101 build steps, no skips. The development environment supplies the grammar
  directory; explicitly setting it also exercises the four grammar-dependent
  native tests skipped by the ordinary build configuration.
- `zig build test-nix -Doptimize=ReleaseSafe -Dcpu=baseline`: true.
- Installed build and installed `tests/ghostty-input.py` PTY checks.
- `tests/settled-frames.py` and `tests/threaded-terminal.py` PTY checks.

The final follow-up updates the Nix catalog expectation and removes a stale
subscription-documentation statement about unfinished rendering work. No further
production change was needed after the dispatcher cutover.

| Requirement                                                        | Evidence                                                                                                                                               |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Ordinary-table immutable updates, structural sharing, no drafts    | `state-patches`, generated `patch-properties`, `state-dispatch`, and bundled reducer ownership tests                                                   |
| UI-independent re-frame-style subscriptions                        | `subscriptions`, `subscription-transactions`, declared query inputs and consumer-owned scopes in `docs/subscriptions.md`                               |
| Open dispatch and no global interception                           | `command-state`, real forward/reverse `routing-state`, `editor-lifecycle`, and rejection of the removed setup API                                      |
| Raw facts separated from presentation                              | `indicator-values`, `component-resolution`, `component-projections`, `choice-preview`, `cost-state`, native response-metadata fixture                  |
| Theme, model picker, hover-only backgrounds                        | `model-affordances`, input-layer component tests, installed Ghostty PTY click/key/motion/leave checks                                                  |
| Structural navigation, visual ranges, Markdown, non-copy consumers | `selection-state`, transcript interaction fixture, installed PTY `v` range checks                                                                      |
| Claude tool-call deduplication                                     | `claude-stream-state` streamed/final record fixtures and batching/ownership assertions                                                                 |
| Coding-plan usage for all three providers                          | `claude-usage`, `codex-usage`, `kimi-usage`, shared `usage-dashboard`, plus the dated live native retrieval checks below                               |
| Startup and streaming/rendering costs                              | Recorded AOT and mixed-transcript evidence in `benchmarks/architecture-handoff-2026-09-07.md`; current selective recomputation and settled-frame tests |
| Small reusable property-test generators                            | `tests/generators.lua`, generator composition/replay/shrinking tests, and generated patch/editor/routing/selection/stream cases                        |

The Nix API evaluation now includes both shared presentation modules; it checks
the standard extension catalog and exported configuration/module contracts.
The installed profile and explicit source override are exercised by the native
integration suite. Separate PTY checks cover coherent settled frames and terminal
progress during blocked Fennel, ordered input, resize, and shutdown.

Performance numbers retain their recorded source revision and measurement scope;
they are not claims about emulator painting, interactive startup, O(1) streaming,
or a universal 16 ms deadline. Live usage probes are also identified by revision;
the current suite uses deterministic provider fixtures, not paid inference.
No release, tag, push, annotation workflow, or conversation-fork workflow was
part of this handoff. Those future workflows remain separate from the reusable
selection and subscription architecture.

## Implementation history

The dated audit notes and the per-migration evidence moved to
[docs/history/architecture-audit.md](docs/history/architecture-audit.md). They
are ordered as the work landed and later entries supersede earlier ones; this
document keeps the accepted scope, the checklist, and the handoff evidence.
