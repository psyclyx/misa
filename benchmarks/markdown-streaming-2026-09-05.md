# Incremental Markdown, 2026-09-05

The baseline already contains the earlier anchored-search fix. Before changing
it, the existing public parser harness measured 16 KiB plain text at 6.247 ms
median / 6.157 ms best, and mixed Markdown at 5.250 / 5.161 ms (10 samples).
Inspection found grammar dispatch on every ordinary byte, and full parsing and
layout on every redraw. This change scans ordinary inline runs in one operation,
retains earlier blocks during appends, and caches layout outside the parser.

The streaming harness compares the old full parser, the new full parser, and
the new incremental document API. This separates the scanning improvement from
incremental reuse. Each run feeds 128-byte chunks, keeping only the latest
document, as the UI does. The redraw workload parses once and repeats the same
source 127 times. Reported times are total parser CPU seconds per complete run,
not per update or end-to-end frame latency.

| Workload | Updates | Old full median / best (s) | New full median / best (s) | Incremental median / best (s) |
| --- | --- | --- | --- | --- |
| Paragraph, 16,500 bytes | 129 | .386323 / .384476 | .045986 / .045833 | .046114 / .045704 |
| Headings/paragraphs, 16,800 bytes | 132 | .409049 / .407054 | .190119 / .185896 | .014056 / .013942 |
| One fence, 17,110 bytes | 134 | .030389 / .030005 | .029636 / .029140 | .029607 / .029013 |
| One table, 16,830 bytes | 132 | .528594 / .527163 | .300789 / .298620 | .299959 / .297453 |
| Unchanged redraws, 16,500 bytes | 128 | 1.027895 / 1.020415 | .432707 / .428349 | .002675 / .002615 |

The multi-block stream improves about 29× overall, including a 13.5× improvement
over the new full parser. Single active blocks still parse as a unit: incremental
reuse adds no meaningful gain for the paragraph, fence, or table workloads.
The fence differences are small and are not claimed as an improvement.
Whole-source normalization/prefix checking and the active tail remain work
proportional to their input sizes. This is block-level incrementality, not
constant-time token updates or a worst-case linear Markdown guarantee.

LuaJIT from the project environment, default JIT enabled; `os.clock` timing,
10 samples per variant, alternating baseline/full/incremental and reverse order.
Builds and other agent probes were stopped for the final measurement. No CPU
affinity or system-wide cache manipulation. Before timing, six unchanged
baseline stream runs matched; both new paths matched the complete baseline AST
at every prefix. Timed runs check the final AST and a source/block-count digest.
The saved CSV contains every timed sample. An initial harness retaining every
historical AST increased heap pressure; it was replaced before the recorded
measurement with the latest-document workload above.

Regression coverage checks byte-split Markdown, CRLF, Unicode, replacements,
truncation, stable block identity, old-snapshot immutability, width/style changes,
layout parity, and cached-span isolation from theme resolution. A separate
10,000-stream adversarial probe passed under tight block/source/inline limits.
The prior 1,009-input full-parser AST parity corpus also passes. Animation tests
cover enabled/disabled motion, static choices, shared timers, swaps, persisted
selections, idle shutdown, stale ticks, and restart phase.

Reproduce the archived baseline and benchmark:

```sh
# Apply the historical patch to the original Lua candidate from this measurement.
patch -R -o /tmp/markdown-baseline.lua /path/to/original/markdown.lua < benchmarks/markdown-incremental.patch
tools/fennel benchmarks/markdown-streaming.fnl /tmp/markdown-baseline.lua extensions/markdown.fnl
tools/fennel benchmarks/markdown-parity.fnl /tmp/markdown-baseline.lua extensions/markdown.fnl
```

Layout reuse is verified by identity and output-parity tests, not included in
the parser timings above. Cached message bodies are released on transcript reset.
No implementation variants were discarded; the measured fence neutrality is
retained because it uses the same parser as the faster multi-block path.
