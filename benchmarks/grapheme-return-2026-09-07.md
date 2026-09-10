# Remove a closure allocation from every grapheme measurement

Baseline: `08ce979`. The dominant clipping stack led to the compiled Lua for
`cluster` in `extensions/layout.fnl`. Fennel compiled this return:

```fennel
(values next-at (if emoji (math.max cells 2) cells))
```

into a freshly constructed function capturing `emoji` and `cells`, called to
produce the final return value. Binding the scalar before `values` removes that
closure. This changes neither segmentation nor width policy, adds no cache, and
does not modify the compiler or specialize ASCII handling.

## Correctness and allocation

`benchmarks/layout-parity.fnl` compares every codepoint width from 0 through
0x10ffff and 500 generated mixed-text cases against the saved baseline. It passed
for clip/take return values, widths, cursor boundaries and rich-span wrapping.
The cases include combining marks, ZWJ emoji, flags, Indic conjuncts, controls,
malformed UTF-8 and narrow widths.

`benchmarks/grapheme-allocation.fnl` makes six warm-up batches, then ten
alternating A/B and B/A allocation observations. Each observation measures 64
width calls. GC is stopped only for this allocation probe, never for native
frame timing. All returned widths must match the baseline.

Median allocated KiB per 64 width calls (not resident or peak memory):

| Input                             | Baseline | Candidate |
| --------------------------------- | -------: | --------: |
| `ASCII ` repeated 64 times        | 3648.203 |     0.203 |
| `é界👩‍💻🇺🇸क्ष` repeated 64 times    | 3040.203 |     0.203 |
| `text é 界 👩‍💻 ` repeated 64 times | 6688.203 |     0.203 |

Candidate observations occasionally included less than 1 KiB of additional
runtime overhead. The removed allocation scales with grapheme count; the tiny
remaining observation is not a claim that the entire measurement harness is
allocation-free.

## Native comparison

Six unchanged 300-block streaming baseline runs produced identical complete
frame hashes before editing. Ten alternating A/B then B/A pairs subsequently
ran all six workloads using the same ReleaseSafe baseline-CPU executable and
different extension directories. Each workload has six warm-ups and ten measured
frames. `--rest-ms 25` clears the presenter's pacing interval. No builds, tests,
profiles or other agent-run benchmarks ran concurrently. Complete frame hashes
matched in every run, alongside the stream-content, sharing and OSC checks.

Wall milliseconds per frame: **median / best of the ten run medians**, not
individual-frame percentiles:

| Blocks | Workload |      Baseline |     Candidate |
| ------ | -------- | ------------: | ------------: |
| 1      | Redraw   | 1.268 / 1.134 | 1.063 / 0.914 |
| 1      | Stream   | 2.293 / 2.151 | 1.821 / 1.734 |
| 16     | Redraw   | 2.118 / 1.912 | 1.578 / 1.487 |
| 16     | Stream   | 3.445 / 3.079 | 2.600 / 2.429 |
| 300    | Redraw   | 2.410 / 2.086 | 2.008 / 1.895 |
| 300    | Stream   | 4.603 / 3.848 | 3.711 / 3.152 |

## Longer streaming observation

The native harness now appends inclusive empirical p95/p99 estimates to its CSV;
the sample count remains explicit. Separate 1,000-frame streaming observations
at 300 blocks, candidate first, produced matching complete frame hashes:

| Variant   | Median | Best individual frame |    p95 |    p99 | Maximum |
| --------- | -----: | --------------------: | -----: | -----: | ------: |
| Baseline  |  4.210 |                 3.065 | 14.770 | 17.246 |  20.075 |
| Candidate |  3.073 |                 2.413 |  7.544 | 13.703 |  15.140 |

All 1,000 measured candidate frames were below 16.7 ms in this observation.
This is not another interleaved A/B study or proof of an every-frame guarantee.
The harness still uses a 25 ms idle gap and one synthetic response with Markdown
blocks, and measures native processing through PTY output, not GUI painting.
Sustained-input and representative mixed-tool/multi-response workloads remain
needed before closing the rendering-performance requirement.

The aggregate allocation improvement is established separately from native
latency: GC stack samples alone would not identify the allocation's owner. The
compiled closure, allocation scaling and isolated frame comparison agree on
the mechanism here.

## Verification

The full ReleaseSafe baseline-CPU suite and installed build passed. Inspection of
the installed Lua confirms a scalar assignment and direct two-value return,
with no per-call helper function. The installed-catalog PTY initially exposed a
test-reader race: it accepted the tail of an earlier frame, starting with a style
reset and ending with the synchronized-frame terminator, as the hover frame.
A diagnostic rerun reproduced that fragment followed by a complete frame with
the correct hover background and OSC link. The reader now requires both frame
delimiters rather than an arbitrary terminator. Partial-prefix and older-frame
tests cover the parser, and 60 consecutive installed-catalog PTY runs passed.
The application's hover implementation was not changed.
The corrected reader also passed against worktree extensions and the previously
built Nix package. A one-sample native invocation verified that the newly added
p95/p99 fields both equal that sole observation.
