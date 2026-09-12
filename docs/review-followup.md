# Review follow-up plan

A code review of the tree at `4908bf8` produced the findings below. Each slice is
independently landable and states its verification. Nothing here is a
prerequisite for the others unless noted.

Baseline evidence: `zig build test` at the review revision, with an isolated
cache so a developer's live `zig build run` session is not disturbed.

```sh
zig build test --cache-dir /tmp/mc --global-cache-dir /tmp/mcg --prefix /tmp/mp \
  -Dtree-sitter-dir="$MISA_TREE_SITTER_DIR" --summary all
# Build Summary: 229/229 steps succeeded; 292/292 tests passed
```

Standalone Fennel policy tests need no Zig build, and the optional PTY
regressions run against the fixture executable:

```sh
sh tools/fennel tests/indexed-patches.fnl
python3 tests/policy-fault.py zig-out/bin/misa-fixture
```

## Status

| Slice | Scope                                                        | State    | Evidence                                                    |
| ----- | ------------------------------------------------------------ | -------- | ----------------------------------------------------------- |
| 1     | Contract documentation matches the code                      | done     | review                                                      |
| 2     | Registration and view-layer diagnostics                      | done     | suite                                                       |
| 3     | Indexed and appended state patches                           | done     | `tests/indexed-patches.fnl`, `patch-controls` case          |
| 4     | Recoverable policy faults with an interactive fault boundary | done     | `tests/policy-fault.py`                                     |
| 5     | Documentation generation and repository hygiene              | done     | `tools/generate-docs.fnl`, catalog/mirror checks            |
| 6     | Retry for transient provider transport failures              | done     | `tests/http-cancellation.py` retry modes                    |
| 7     | Extension discovery outside `MISA_EXTENSION_DIR`             | done     | runtime case in `tests/integration/runtime.zig`             |
| 8     | Wire the unwired capability: conversation log and `/resume`  | done     | `tests/conversation-state.fnl`, `conversation-read` case    |
| 9     | Presentation cost: window-scoped projection                  | measured | `benchmarks/projection-phases.fnl`; no change made, and why |
| 10    | Single native implementation of terminal cell layout         | done     | `tests/native-layout.fnl`, extended parity case, frame A/B  |

The suite grew from 292 cases at the review to 325. Slice 4's interactive behavior
and every PTY regression are separate Python checks, because the fixture harness
runs headless.

## Verified findings behind the plan

Details and file references are in the review. The short version:

- The README's fixed-effect list and catalog table had drifted from the
  implementation: `operation/finish` and `file/edit_lines` were missing,
  `view-layers` entries were described as bare functions, and the
  `serializers` catalog was undocumented.
- `misa.standard_extensions.entries` and `tests/stock.fnl` are hand-maintained
  mirrors of data that the filesystem already determines; the Nix catalog
  (`nix/standard-extensions.nix`) is the only one that derives itself.
- The state model cannot express an incremental array update, so every list
  update copies. `extensions/misa/transcript/model.fnl` copied the whole block
  array and the whole chunk array per stream delta.
- Faults in a _model_ handler or an invalid native effect ended an interactive
  session, while faults in a _view_ or a _component_ were contained. The
  dispatch-chain limit already implemented the interactive/headless split.
- A layer that returned neither `exclusive`, `overlay`, nor `dock = :input` was
  dropped silently.
- `src/conversation/root.zig` (a SQLite store with forks, cost accounting and
  blob dedup), the `json/decode` effect, the `file/edit` effect, and six
  services have no caller in the shipped application.
- Terminal cell width and grapheme segmentation exist twice, in
  `src/terminal/width.zig` and `extensions/misa/ui/layout.fnl`, without a
  cross-checking test in the suite.

## Slice 1 — contract documentation (done)

- Documented `operation/finish` and the `file/edit_lines` anchored edit form,
  and the `file/read` window arguments.
- Added `serializers` to the catalog table and corrected the `view-layers` row.
- Documented `misa.at` and `misa.append`, the `keys` action shorthand, the
  state-file path precedence, and that an unrecognized catalog kind is still
  installed and readable through `misa.catalog`.

## Slice 2 — diagnostics (done)

- Every registration assert in `src/lua_runtime/framework.fnl` carries a
  message, including the event type, cofx name, or effect type it rejected.
- `misa.ui.layers` carries each layer's registered ID, and root composition
  rejects a layer that declares no role instead of dropping it.

## Slice 3 — indexed and appended state patches (done)

- `misa.at(index, value)` writes one array element (bounded by length plus one)
  and returns the input array when the write changes nothing.
- `misa.append(value)` grows an array. Appending is the one non-idempotent
  patch control, and is documented as such.
- Both reject a nil value, a non-integer or out-of-range index, a non-array
  target, and a control nested inside replacement data.
- `block-index`, `replace-transcript-block`, `finish-streaming-block`, and the
  two streaming chunk accumulators use them: a delta no longer rebuilds the
  block array or the chunk vector twice.
- `tests/indexed-patches.fnl` covers the contracts plus a generated
  in-range/out-of-range property, and `tests/integration/configs/patch-controls.fnl`
  exercises both controls through a real dispatched transaction.

## Slice 4 — recoverable policy faults (done)

- A failed model transaction (handler error, invalid handler result, invalid or
  unknown native effect) is rolled back and reported as
  `runtime/handler-error` or `runtime/effect-error` with `event_type` and
  `text`, and the interactive session keeps reading input.
- A headless run keeps its previous exit status and diagnostic; an invalid
  effect still names its type through `effectType`.
- A rejected frame reports `runtime/presentation-error` once per episode
  (reporting every attempt would keep the queue non-empty), keeps the last
  valid frame, and retries after a later event.
- `extensions/misa/transcript/init.fnl` exposes one `runtime-fault` handler for
  all four runtime notices, and the standard transcript wiring registers them.
- The fixture's fault path re-arms `read_requested`, matching the existing
  dispatch-limit behavior; without it a contained fault would leave the session
  unable to read the next key.
- `tests/policy-fault.py` fails against the previous binary ("session failed:
  UnknownNativeEffect" before any notice) and passes with the change.

## Slice 5 — documentation generation and hygiene (done)

- `tools/generate-docs.fnl` installs the stock application and writes
  `docs/catalogs.md` and `docs/services.md`; the suite runs it in `--check` mode
  so a stale document fails the build. The generated documents are excluded
  from the formatter, which would otherwise rewrite them and break that check.
- The bundled catalog is compared with `find extensions -name '*.fnl'`, captured
  by `build.zig` as a build input, in both directions.
- `tests/extension-composition.fnl` asserts that `tests/stock.fnl` mirrors every
  `misa.standard.*` module except the configuration, builder, and deliberately
  unstocked ones.
- `treefmt.toml` now covers Fennel under `tests/`, `benchmarks/`, `tools/`, and
  all of `src/lua_runtime/`; the 123 files that had drifted were formatted in
  one commit.
- `ARCHITECTURE-WORK.md` keeps the accepted scope, the checklist, and the
  handoff evidence; the dated audit notes moved to
  `docs/history/architecture-audit.md`.

## Slice 6 — transport retry (done)

`http/request` retries a transport failure that leaves the request unjudged and
the transient statuses 408, 425, 429, 500, 502, 503, 504, bounded by
`retries={attempts,backoff_ms,max_ms}` and honoring `Retry-After` up to the
ceiling. Cancellation and timeouts are never retried, and a stream that already
delivered a record is never replayed.

Evidence: three unit tests over the policy helpers and declaration validation,
and two new modes in `tests/http-cancellation.py` that drive the real transport
against a local server (503 then 200 with exactly two requests; 401 with
exactly one).

## Slice 7 — extension discovery (done)

`XDG_CONFIG_HOME/misa/extensions`, otherwise `$HOME/.config/misa/extensions`, is
searched between the configuration's own directory and the installed catalog.
A runtime case writes a plugin there, requires it by name from a configuration
in another directory, and asserts its view is committed.

## Slice 8 — conversation log and `/resume` (done)

The policy in `misa.conversation` journals the canonical messages a settled turn
added, records a reset entry when history was replaced, and can list, load, and
install a stored conversation; `misa.agent` gained the one event that performs
the handover. `tests/conversation-state.fnl` covers the cursor, the reset entry,
page accumulation, picker tokens, and failure reporting; the `conversation-read`
case proves the stored round trip including the new header `metadata`.

## Slice 9 — presentation cost (measured, no production change)

The review predicted that a frame costs the whole transcript. The first
measurement of this slice used redraw frames only
(`benchmarks/native-transcript.py`, Debug build, mixed scenario, six samples:
1 block 3.240 ms, 16 blocks 4.892 ms, 300 blocks 4.887 ms redraw, 7.2–7.7 ms per
stream delta) and concluded that a frame is dominated by fixed work. That
conclusion holds for redraw frames and is wrong for delta frames: a redraw frame
does not replace the block array, so `misa.transcript.project` reuses the
accepted projection and never walks the transcript, while a delta replaces it and
re-renders every block through the per-item cache.

`benchmarks/projection-phases.fnl` separates model dispatch from presentation and
times both phases by transcript size and frame kind. Best milliseconds per frame,
60 iterations (`benchmarks/projection-phases-2026-09-12.md` has the full table and
scope):

| Blocks | Redraw (presentation) | Delta (presentation) |
| -----: | --------------------: | -------------------: |
|      1 |                0.2400 |               0.3930 |
|     16 |                0.3810 |               0.5560 |
|    300 |                0.4300 |               1.0490 |
|   1000 |                0.3730 |               2.2990 |
|   3000 |                0.5080 |               7.9680 |

Redraw presentation is flat, so the per-item cache does its job. Delta
presentation is linear in transcript size at about 2.7 µs per block: 1.05 ms at
300 blocks and 8.0 ms at 3000. Model dispatch is 0.38 ms at 3000 blocks.

Two candidate caches were implemented and rejected earlier on evidence or
soundness, and the new numbers do not revive either:

- Memoizing `misa.syntax.for-model` changed nothing measurable.
- Caching the selection lookup per block is unsound: the selection service is a
  replaceable catalog entry that may read any state, so no key short of the
  database itself is safe, and that key removes the benefit.

No production change was made in this slice. The delta path is the one that
scales with the transcript, and the fix is not a local cache:

- `install-projection` in `src/lua_runtime/framework.fnl` compares a projection's
  declared inputs field by field and reuses the accepted value, but it does not
  pass the previous value to `render`, while a computed subscription receives
  `previous` in its `compute`. A projection therefore cannot retain per-item work
  across a recompute, which is exactly what a delta needs: all of its inputs
  except the block array are unchanged, so the items for unchanged blocks could
  be reused verbatim. Extending the projection contract to pass the previous
  value would be consistent with subscriptions and is the smallest enabling
  change.
- Two hazards have to be answered first. Reusing an item is only sound for the
  state the projection already declares as an invalidation input, so any item
  input that is _not_ declared (attachments, for instance) would be reused
  stale — the projection's input list has to become the authoritative statement
  of what an item depends on. And the item list interleaves group headers and
  footers, so reuse has to key on structure (owner transitions and item roles),
  not on position, or a changed block silently shifts every later row.
- Window-scoped assembly is the other half: the delta still materializes one line
  array for the whole transcript, and the viewport slices 32 rows out of it. That
  change needs a per-block line index plus a row-range materializer, and it
  touches `presentation.project`, `transcript.viewport`, and document geometry.

Both are refactors of the presentation contract rather than caches, and neither
is justified until a transcript of a few thousand blocks is a real workload. The
measurement above is the input to that decision; nothing in the suite regressed
while it was taken.

## Slice 10 — single native cell layout (done)

`src/width/root.zig` (moved out of `src/terminal/` so both the terminal module
and the Lua runtime can import it as `misa_width`) is reachable from Lua as
`misa.native.width`, `misa.native.cell-width`, and `misa.native.clusters`, and
the runtime now also registers that table as the module `misa.native`, so a Lua
projection can `require` it instead of depending on the configuration global.

`extensions/misa/ui/layout.fnl` no longer carries wcwidth interval tables, a
virama list, or a grapheme walk: every primitive reads the native measurement.
The Lua side keeps the byte-cursor arithmetic and the word-breaking policy, and
its own UTF-8 length rule for the editor's newline normalization, which is byte
structure rather than a width table.

The standalone Fennel harness (`tools/fennel`) has no native module, so it
registers `tests/native-layout.fnl` as `misa.native`: the previous Lua tables and
walk, kept as the offline stand-in. The layout-parity integration case compares
that stand-in with the native module over every codepoint, the grapheme corpus,
and the returned cluster arrays, so the two cannot drift; without either module
`misa.ui.layout` fails to load instead of measuring text differently.

Evidence (`benchmarks/native-layout-2026-09-12.md` has the tables and scope):

- `zig build test` passes (235/235 steps, 325/325 tests, including the extended
  layout-parity case) and `zig build test-nix` succeeds.
- `benchmarks/layout-parity.fnl` compares the previous implementation with the
  new one over every codepoint width and 500 generated mixed-text cases through
  `clip`, `take`, `width`, the three boundary walks, and `wrap-spans`.
- All 77 standalone Fennel cases, the settlement, threaded terminal, policy
  fault, and Ghostty PTY regressions pass.
- `benchmarks/native-transcript.py --extension-dir` runs the same executable
  against either layout; both variants produce byte-identical frames, and frame
  medians move in both directions within the noise of the machine.
- `misa.layout.width` allocates nothing in either variant. A delta frame of the
  300-block fixture allocates 674 KiB instead of 615 KiB, because a cluster walk
  returns one flat table; the 6000-byte-line and 270 000-byte-message regression
  is unchanged (1089 ms vs 1088 ms).
