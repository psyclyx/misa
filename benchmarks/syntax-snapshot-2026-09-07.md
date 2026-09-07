# Resolve the syntax collection once per transcript projection

Baseline: `0f7c10c` extension tree. Candidate splits collection subscription access
from pure per-model source matching. It does not change state validation, cache
ownership, snapshot data, Markdown semantics, or the native executable.

Ten alternating A/B then B/A pairs used `benchmarks/native-transcript.py` with
`--extension-dir` pointing to each source tree and `--rest-ms 25`. Each invocation
ran all six workloads with six warm-ups and ten measured frames. Complete frame
SHA-256 values matched across every invocation/variant for each workload, in
addition to the benchmark's state, sharing, stream-text and OSC checks. No builds
or other benchmarks ran concurrently.

Wall milliseconds: median / best **of the ten run medians**, not individual-frame
percentiles or minimum individual-frame latency:

| Blocks | Workload | Baseline | Candidate |
| --- | --- | ---: | ---: |
| 1 | Redraw | 1.345 / 1.197 | 1.382 / 1.225 |
| 1 | Stream | 2.314 / 2.116 | 2.281 / 2.058 |
| 16 | Redraw | 2.207 / 2.021 | 2.211 / 2.065 |
| 16 | Stream | 3.890 / 3.653 | 3.716 / 3.317 |
| 300 | Redraw | 5.162 / 4.512 | 4.507 / 3.961 |
| 300 | Stream | 9.995 / 8.795 | 7.571 / 6.828 |

The main mechanism is eliminating 299 repeated query-key/dependency evaluations
per 300-block transcript projection. The same immutable collection is now read
once; model lookups remain source-checked. Tests explicitly count one collection
read for 300 blocks and reject any subscription access from the pure lookup.
Existing async completion, retained-state and rollback tests remain applicable.

The first session's one-block redraw difference is small in absolute terms but
is recorded rather than hidden. A fresh ten-pair check, starting with the opposite
variant order, reversed it: baseline 1.407 / 1.289 ms versus candidate
1.375 / 1.206 ms (again median / best of run medians), with matching frame hashes.
There is no consistent one-block regression established by these sessions.
This change does not prove all native frames meet
16.7 ms, and does not remove full block-vector replacement validation or Unicode
clipping costs identified in the earlier profile.

Verification: focused syntax/state/component-projection tests, the complete
ReleaseSafe baseline-CPU suite, installed build, and Ghostty-input PTY regression
passed. The API is a clean cutover: `syntax_projection` now takes a collection
snapshot, not the full database; bundled callers and documentation are updated.
