# Rejected: skip width-table searches below the first range

Baseline: `a60b6e8` extension tree. The candidate changes only the private interval
membership helper: a codepoint below the first sorted range cannot be present,
so the binary search starts with an empty search interval. The bound comes from
the existing table, not an ASCII-specific constant. There is no cache, alternate
grapheme algorithm, changed width table, or relaxed validation.

Correctness probe:

```sh
tools/fennel benchmarks/layout-parity.fnl /path/to/baseline/extensions/layout.fnl
```

It checks every codepoint width from 0 through 0x10FFFF against the baseline and
500 generated mixed-text cases at widths 0, 1, 2, 5, 16 and 80. It compares all
clip/take return values, widths, cursor boundaries and rich-span wrapping.
Inputs include combining characters, emoji/ZWJ/flags/keycaps, Indic conjuncts,
controls and malformed UTF-8. All checks passed.

Native timing uses the same executable and the two extension trees selected with
`benchmarks/native-transcript.py --extension-dir ... --rest-ms 25`. Ten pairs
alternate A/B and B/A order. Each invocation covers 1/16/300-block redraw and
streaming, six warm-ups and ten measured frames per workload. Complete frame
hashes must agree for every workload across all runs; state/sharing and current
stream-content checks remain enabled. No builds or other benchmarks run
concurrently. These are keyboard-to-completed-frame wall timings, not CPU time or
terminal-emulator painting latency.

Results in wall milliseconds. The median column is the median of ten run
medians; best is the fastest individual measured frame across those runs:

| Blocks | Workload | Baseline median / best | Candidate median / best |
| --- | --- | ---: | ---: |
| 1 | Redraw | 1.357 / 0.795 | 1.402 / 1.041 |
| 1 | Stream | 2.364 / 1.599 | 2.307 / 1.630 |
| 16 | Redraw | 2.235 / 1.599 | 2.122 / 1.666 |
| 16 | Stream | 3.671 / 2.913 | 3.547 / 2.690 |
| 300 | Redraw | 4.153 / 3.382 | 4.125 / 3.402 |
| 300 | Stream | 7.242 / 6.241 | 7.550 / 5.799 |

Frame hashes matched throughout, but this does not establish a whole-application
win. The target 300-block streaming median was worse and small-workload results
were mixed. The production edit was reverted. No claim is made about a repeatable
regression mechanism or the cost of isolated interval lookup; the evidence is
insufficient to ship it as an optimization. The broad parity probe remains useful
for future layout changes. This rules out only this particular extra lower-bound
branch, not all improvements to Unicode clipping.

To reconstruct the rejected experiment, the sole code change in the private
`contains` helper was:

```fennel
;; baseline
(var high (length intervals))
;; rejected candidate
(var high (if (< cp (. intervals 1 1)) 0 (length intervals)))
```
