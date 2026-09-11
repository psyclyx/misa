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

| Slice | Scope                                                         | State   | Evidence                             |
| ----- | ------------------------------------------------------------- | ------- | ------------------------------------ |
| 1     | Contract documentation matches the code                       | done    | review                               |
| 2     | Registration and view-layer diagnostics                       | done    | 294/294 suite                        |
| 3     | Indexed and appended state patches                            | done    | `tests/indexed-patches.fnl`, 294/294 |
| 4     | Recoverable policy faults with an interactive fault boundary  | done    | `tests/policy-fault.py`, suite       |
| 5     | Documentation generation and repository hygiene               | planned | —                                    |
| 6     | Retry for transient provider transport failures               | planned | —                                    |
| 7     | Extension discovery outside `MISA_EXTENSION_DIR`              | planned | —                                    |
| 8     | Decide the fate of unwired native capability                  | planned | —                                    |
| 9     | Presentation cost: window-scoped projection and a keyed cache | planned | —                                    |
| 10    | Single native implementation of terminal cell layout          | planned | —                                    |

The suite grew from 292 to 294 cases: one standalone patch-control program and
one runtime case. Slice 4's interactive behavior is additionally covered by a
PTY regression, because the fixture harness runs headless.

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

## Slice 5 — documentation generation and hygiene

Deliverable: generated catalogs and services reference; formatter coverage.

- Generate `docs/catalogs.md` and `docs/services.md` from an installed stock
  application.
- Derive the standard-extension catalog and `tests/stock.fnl` from the
  filesystem rather than restating it.
- Extend `treefmt.toml` to Fennel under `tests/` and `benchmarks/`, then
  reformat once (99 files under the two directories are currently not
  fnlfmt-clean, and `src/lua_runtime/*.fnl` fixtures are covered by a narrower
  glob than they appear to be).
- Split `ARCHITECTURE-WORK.md` into current architecture and dated history.

## Slice 6 — transport retry

Deliverable: bounded retry with backoff for transient provider failures.

- Retry connection failures, HTTP 408/429/5xx and truncated streams, honoring
  `Retry-After`, bounded by attempts and the effect's `overall_ms`.
- Never retry after response bytes have been committed to the transcript, and
  never retry a credential or write effect.

## Slice 7 — extension discovery

Deliverable: third-party extensions install without `MISA_EXTENSION_DIR`.

- Search `$XDG_CONFIG_HOME/misa/extensions` (then `$HOME/.config/...`) after
  the installed catalog, plus an explicit `config.extensions` array.
- Report a load error that names the module and the searched roots.

## Slice 8 — unwired capability

Deliverable: each capability is either reachable or removed.

- Conversation store: add `conversation/load`, `conversation/list` and a
  `/resume` command, or delete the module and its SQLite dependency.
- `json/decode` and `file/edit`: remove, or document them as supported.
- The six unread services: remove or document as extension-facing API.

## Slice 9 — presentation cost

Deliverable: a frame's cost follows the viewport, not the transcript.

- Project and render only the blocks intersecting the viewport, reusing stored
  per-item geometry.
- Keep more than one projection entry per definition, keyed by the context
  fields a projection declares.
- Reuse owned output lines for unchanged items.

## Slice 10 — native terminal cell layout

Deliverable: one width and grapheme-segmentation implementation.

- Expose the native implementation as a service, delete the mirrored Lua
  tables, and add a parity test to the suite (the existing
  `benchmarks/layout-parity.fnl` needs a saved baseline file and is not run by
  `zig build test`).
