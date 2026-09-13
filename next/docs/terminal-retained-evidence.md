# Terminal retained presentation evidence

The terminal consumes validated canonical operations and separate stream updates. Section
layout is split into retained line owners; content kinds such as quotations, lists and
closed disclosures own their nested rendering. A canonical operation formats only its
affected owner. Stream append touches appended characters and a row index, then copies
only visible rows into the terminal output writer.

## Deterministic work experiment

Run from `next/`:

```sh
cargo test -p misa-tui append_formatting_and_viewport_copy_work_do_not_grow_with_history -- --nocapture
```

The fixture uses a 40-column, 16-row screen, 100 or 1,000 settled messages, and five
appends totaling 21 UTF-8 bytes, including wide characters and a newline. Each size runs
six times. The previous `draw` path remains the baseline; a test-only counter increments
inside every actual recursive `Screen::resolve` call. Each baseline size produced one
identical output across its six repetitions. The retained results also repeated exactly.

Counts below are totals for five appends, not per-append values:

| Settled messages | Baseline resolve visits | Retained canonical nodes formatted | Appended bytes consumed | Visible rows copied | Counted row-index steps |
| ---------------- | ----------------------: | ---------------------------------: | ----------------------: | ------------------: | ----------------------: |
| 100              |                   1,025 |                                  0 |                      21 |                  70 |                      40 |
| 1,000            |                  10,025 |                                  0 |                      21 |                  70 |                      55 |

The last column counts Fenwick lower-bound and update steps. Prefix-sum reads are not
included in that counter; they also take logarithmic work. These are component work
counts, not elapsed-time measurements or claims about total process cost. Builds ran
concurrently elsewhere, so no timing comparison is reported.

## Correctness gates

- Six repeated operation sequences compare all settled styled lines with the full renderer
  after every insert, replace and remove, including section-to-content changes and width
  invalidation. Separate coverage exercises edits inside a closed disclosure and a quote.
- Live text uses character wrapping. Selection and copy consume the exact retained rows,
  including the width-10 `hello world` case whose last displayed row is `d`.
- Chrome, staged attachments and multiline editor content are physical rows. A full-height
  transcript plus staging and a multiline draft remains within the screen, with a cursor
  matching the displayed editor row and column.
- Startup, completion and save waits retain keyboard, paste, resize and quit liveness.
- A real endpoint test verifies that rejected drafts are returned with their references,
  late correlated faults become notices, and a subsequent prompt is acknowledged normally.
- Clipboard tests wait for upload/send acknowledgments: staging does not submit, failed
  submission preserves the exact draft and references, success consumes those references,
  discard is local, and an image-only Alt-Enter sends `Interrupt` with its staged reference.

Verification on the merged main snapshot: 59 terminal tests pass; the native clipboard
roundtrip test is ignored without a display and remains covered by the artifact clipboard
check. `cargo check -p misa-tui` passes without warnings.

## Cost boundaries

Snapshots, width/theme/disclosure changes and explicit local interaction may materialize
or format the full canonical document. Structural canonical changes update the cached
owner order and row-index metadata, which currently scans that order. They do not format
unaffected history. Ordinary stream appends neither snapshot nor scan settled history.
Selection-body materialization happens on explicit selection movement; passive stream
repaint reuses it. The terminal output writer compares only the bounded visible rows.

Request state is bounded: 16 ordinary pending requests/transfers, with room up to 32 for
control intents; the UI request queue holds 16 and its presentation queue holds one.
The keyboard reader holds 64 queued events plus one pending event. No reply wait keeps
an unbounded transcript inbox. Prompt success is correlated with server `Ack`, while
completion, save and upload replies arrive independently of view delivery.
