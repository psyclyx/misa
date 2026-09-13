# Publish syntax deltas, not whole syntax snapshots

Baseline extension tree: `bf03f9c`. Candidate changes only syntax processing:
request construction returns data, model updates describe document/pending-request
patches, and the interceptor applies those patches without replacing all syntax
state. Completion patches remove the matching pending request and update only its
document's slots/revision. Unchanged input returns the original transaction.
State validation, the patch API, transcript storage and rendering are unchanged.

## Native comparison

Both variants used the same ReleaseSafe baseline-CPU executable, with
`benchmarks/native-transcript.py --extension-dir ... --rest-ms 25`. Ten alternating
A/B then B/A pairs (starting with the candidate) ran all six workloads, six warm-up
frames and ten measured frames per invocation. No builds, tests, profiles or other
agent-run benchmarks ran concurrently. Complete frame SHA-256 values matched
across all runs and both variants, including the pre-experiment oracle. The
fixture also checks stream contents, retained block identity and OSC links.

Wall milliseconds per frame: **median / best of the ten run medians**, not
individual-frame percentiles:

| Blocks | Workload |      Baseline |     Candidate |
| ------ | -------- | ------------: | ------------: |
| 1      | Redraw   | 1.248 / 1.175 | 1.302 / 1.106 |
| 1      | Stream   | 2.286 / 2.054 | 2.323 / 1.996 |
| 16     | Redraw   | 2.151 / 2.027 | 2.141 / 1.879 |
| 16     | Stream   | 3.855 / 3.433 | 3.758 / 3.367 |
| 300    | Redraw   | 4.206 / 3.790 | 3.245 / 2.204 |
| 300    | Stream   | 7.839 / 7.139 | 4.474 / 3.882 |

The 300-block streaming run median improved in every pair. One-block redraw
has a small unfavorable aggregate difference. A fresh ten-pair check with the
opposite starting order measured baseline 1.279 / 1.238 and candidate
1.339 / 1.190 ms. In each session, five pairs favored each variant: the paired
measurements do not establish a consistent small-workload direction. These raw
numbers are retained rather than claiming all workloads improved.

A separate longer run, candidate first, used 200 measured streaming frames at
300 blocks. Output hashes matched:

| Variant   | Median | Best individual frame | Maximum |
| --------- | -----: | --------------------: | ------: |
| Baseline  |  8.030 |                 6.332 |  22.204 |
| Candidate |  4.299 |                 3.194 |  18.926 |

That longer run is a tail-latency observation, not another interleaved A/B study.
The maximum still exceeds the 16.7 ms frame budget. This change does **not** prove
every production frame meets that budget. The fixture is still one response with
300 Markdown blocks, not a representative mixed-tool/multi-response transcript;
it measures native processing through PTY output, not GUI painting.

## Attribution

The earlier three-frame stack display hid the owner of recursive materialization.
`transcript-profile.fnl` now accepts stack depth (default 12) and an extension
directory. Separate 200-update, 16-level Lua profiles collected 1,544 baseline and
842 candidate samples. Baseline had 616 samples whose stack included
`materialize`, 602 also under an `after` callback; candidate had 7 and 1
respectively. These are sampled stack counts, not native latency percentages.
Source inspection and the isolated extension comparison identify the removed
whole-syntax replacement as the mechanism.

Unicode clipping and transcript projection now dominate the visible stacks.
GC samples identify where collection ran, not which operation allocated garbage.
The transcript block-vector replacement remains unchanged and is still a cost;
it was not the principal recursive traversal in this workload.

## Rejected entry-merge experiment

Before locating the syntax snapshot cost, an explicit `misa.merge(fields)` control
was prototyped so a reducer could update numeric entries without replacing a
vector. It preserved full replacement validation and passed generated nested
patch/rollback tests. Ten alternating native pairs with only this experiment
measured 300-block streaming baseline 7.636 / 6.856 versus candidate
7.777 / 7.075 ms (median / best of run medians). One-block streaming measured
2.226 / 2.020 versus 2.347 / 2.088 ms. All frame hashes matched, but there was no
established native win. A combined experiment was also measured, then discarded
in favor of the syntax-only comparison above.

No new merge control, framework API, or transcript storage change is retained.
The performance question does not currently justify that extra API surface.
The independent patch-key test correction is commit `1f39336`: Fennel `{[1] ...}`
uses a table key, so the old sparse/mixed-key tests were not exercising numeric
keys at all. They now use actual numeric keys.

## Verification

Focused syntax/state tests cover 300 documents, unique request IDs, unchanged
transaction identity, other-document sharing, completion coalescing, retained
state, and source changes. Existing generated syntax sequences and async
regressions continue to pass. Final full-suite, installed-build and PTY results
are recorded in `docs/history/architecture-audit.md`.
