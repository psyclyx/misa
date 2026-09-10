# Streaming input delay, 2026-09-09

The terminal polls input first between settled dispatch chains. The delay was
synchronous work inside those chains: a 32-record transport batch generated one
agent event and one transcript event per chunk, each projecting, decoding, and
validating a view. Only the last view was presented.

The initial Lua profile found recursive copying of already-rendered spans and
full-transcript anchor construction on every projection. Transcript ownership
now reuses immutable component rows through weak keys; anchor geometry is
computed for the needed row, searching the full layout only after displacement.
This reduced a synthetic 32-transcript-delta chain from 464 to 219 ms median,
still too slow. Those two exploratory runs are not the final comparison.

The agent now supplies a shared normalizer used by OpenAI, Codex, Anthropic and
Claude adapters. Adjacent plain text/thinking events already in the same native
batch are combined. Different requests/kinds, extra metadata, and every other
effect remain boundaries. No timer, artificial delay, transport buffering,
input reordering, or transaction validation was changed. Deferring validation
until the whole chain completed was rejected: it could execute effects from
an intermediate state whose view should have rejected the transaction.

## Native measurement

ReleaseSafe, baseline CPU; same binary for both extension trees. The public
native transcript harness uses the default UI and real OpenAI protocol/agent
reducers, with a synthetic 300-block transcript and batches of 32 text records.
Timing starts with a keyboard event and ends at its completed synchronized PTY
frame. A 25 ms idle gap is outside timing. This measures the synchronous chain
that delays the next input; it is not a live-network or terminal-paint benchmark.

Each process warms up for six frames, measures ten, and runs one final untimed
frame to validate the last update. The fixture checks exact transcript text,
agent/transcript agreement, unchanged block identities, and visible streamed
text. All four runs have identical completed-frame hashes. The standalone
projection comparison also verifies complete output records over six unchanged
repetitions before ten interleaved samples per variant.

| Order | Variant   | Median / best / max milliseconds |
| ----- | --------- | -------------------------------- |
| 1     | Candidate | 11.165 / 10.107 / 35.396         |
| 2     | Baseline  | 776.806 / 738.841 / 880.770      |
| 3     | Baseline  | 702.727 / 610.401 / 915.581      |
| 4     | Candidate | 11.964 / 9.770 / 29.356          |

No builds or other benchmarks ran concurrently with these measurements. No CPU
affinity or cache manipulation was used. Small tail-latency spikes remain;
ten samples per process do not establish a universal frame-time bound. The
text-heavy batch gains do not apply to batches composed entirely of distinct
tool/metadata events, which deliberately retain their ordering boundaries.

Additional candidate checks passed for one block (12.087 ms median), a mixed
300-block history containing tools and highlighted code (34.220 ms), and the
normal Debug build (27.520 ms median / 21.877 best / 67.261 max on the same
300-block Markdown transport burst). These are single ten-sample validation
runs, not separate A/B improvement claims. The Debug frame hash matches the
ReleaseSafe comparison. `zig build test` passes.

## Reproduce

Build with `zig build -Doptimize=ReleaseSafe -Dcpu=baseline`. Copy `extensions`
to a temporary checkout and reverse `stream-input-2026-09-09.patch` there to
recover the baseline (the patch is relative to its parent directory). Run:

```sh
python3 benchmarks/native-transcript.py zig-out/bin/misa --blocks 300 --mode stream --rest-ms 25 --burst 32 --transport
python3 benchmarks/native-transcript.py zig-out/bin/misa --blocks 300 --mode stream --rest-ms 25 --burst 32 --transport --extension-dir /path/to/baseline/extensions
```

Repeat in reverse order. Omitting `--transport` isolates transcript event costs;
omitting `--burst` retains the existing single-delta workload. The archived
before/after Lua profiles isolate shallow row copies and on-demand anchors,
before ownership reuse and text folding; they are not end-to-end latency.
`stream-projection-2026-09-09.csv` reports CPU seconds
per eight-frame run, excluding output serialization. Its allocation/GC variation
is why the native chain measurement is the primary result.
