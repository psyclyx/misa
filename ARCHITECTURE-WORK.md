# Architecture and interaction work

This tracks the accepted scope while the implementation is in progress.
Completion requires behavioral evidence, not just passing existing tests.

- [x] State: migrate bundled reducers, services, and interceptors to immutable
  updates; remove the transaction draft and whole-database reconciliation.
- [x] Patches: settle empty-table semantics, validate data and controls, preserve
  sharing and rollback, and cover nested replacement/deletion and collections.
- [x] Subscriptions: explicit query dependencies, safe query keys, nil inputs,
  cycle errors, bounded cache ownership, rollback, and UI-independent consumers.
- [ ] Presentation: semantic render data, open dispatch registries, unified
  renderable composition, and subscriptions replacing ad-hoc projections.
- [ ] Rendering performance: verify selective recomputation and streaming costs
  with representative transcripts, preserving interaction metadata through layout.
- [ ] Startup: profile and improve default startup; record reproducible measurements.
- [x] Default theme: remove unwanted angled decoration; model value and picker
  binding must be distinct from semantic labels and correctly presented.
- [x] Hover: pointer motion highlights clickable backgrounds only while hovered,
  with unchanged resting appearance and working click routing.
- [x] Transcript interaction: structural navigation, Vim-style visual ranges,
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

Syntax remains an unresolved dependency boundary (`extensions/syntax.fnl`): its
global hook captures transcript length before every event, detects appended
blocks afterward, and knows response ownership and several transcript event
types. Moving its code to a differently named global hook would not address
that coupling. The transcript owner should expose explicit lifecycle facts for
derived processing; syntax requests/completions stay effects/events, and syntax
projections stay pure subscriptions. Input normalization/routing interceptors
are a separate concern and are not being removed indiscriminately.

The other domain coupling identified by the interceptor audit is editing undo
bookkeeping (`extensions/editing.fnl`): it snapshots the editor before input and
infers the operation from the resulting text afterward. Undo policy needs the
old editor, new editor and operation reason in the same transaction; moving it
to delayed notifications would break that boundary. Startup initialization and
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

| Requirement | Implementation and evidence |
| --- | --- |
| Explicit dependencies | Registration validates static query vectors; dynamic declarations are checked during evaluation. Tests cover constant/static/dynamic computations and invalid declarations. |
| Safe keys | Typed, length-delimited canonical encoding rejects nonfinite numbers, metatables, cycles and invalid vectors. Generated tests compare copied queries and distinguish extended queries. |
| Missing values | Dependency vectors carry `n`; tests distinguish trailing nil from false and verify memoization identity. |
| Cycles/depth/re-entry | Active-query and depth guards reject recursive graphs; tests cover changing-key deep chains and callback attempts to query/clear/close/fork the evaluating scope. |
| Bounded ownership | Per-consumer caches evict to capacity, copy only their index on fork, and clear references on close. Capacity must now be a positive finite integer; public construction cannot inject inherited entries. |
| Transaction isolation | Staged entries publish only after successful evaluation; framework forks commit with state. Lua-failure, explicit rollback, component-cache rollback and native-decoding rejection tests retain committed identities. |
| UI independence | The core imports no renderer or framework globals; standalone scope tests use ordinary data. Explicit consumer tests query and close a separate scope without rendering. |

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
