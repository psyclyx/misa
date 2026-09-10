# Rendered selection source geometry

Exact Markdown source ranges let selection follow the rows and columns emitted
by the existing renderer, including wrapped words, rich spans, list markers and
code gutters. Viewport anchors now retain the painted source byte through
reflow. This is a correctness tradeoff, not a performance improvement.

The baseline parser and Markdown component come from `d413ee8`. Both variants
use the current shared layout service. The workload contains 6,300 bytes of
headings, paragraphs with Markdown emphasis/links/escapes and joined source
lines, lists and tables. It has no fenced code, whose newly numbered appearance
would intentionally differ from the baseline.

| Workload                              | Baseline median | Candidate median |       Added time |
| ------------------------------------- | --------------: | ---------------: | ---------------: |
| 50 streaming updates, 128-byte chunks |        6.120 ms |         6.434 ms |  0.314 ms (5.1%) |
| Initial layout plus 99 cached redraws |        2.667 ms |         3.138 ms | 0.471 ms (17.6%) |

These are total CPU times per workload, not per redraw, terminal latency or
end-to-end input latency. Compilation, setup, serialization and oracle checks
are excluded. Six repeated runs of each variant passed the same painted-output
oracle, which ignores new source metadata and merges equivalent adjacent spans.
Ten timed samples alternated baseline/candidate order after warmup, collecting
Lua garbage before each sample. See the companion CSV for all samples.

Source maps contain contiguous ranges, not per-character records. Joined
Markdown input contributes ranges only at source-line boundaries. Looking up
those ranges uses binary search. Directional navigation requests only the
selected document through the same component renderer and a separate projection
collection, preserving the ordinary transcript cache and avoiding unrelated
history. This benchmark measures parser/layout overhead, not navigation's range
lookup or complete transcript rendering.

Reproduce from the repository root:

```sh
git show d413ee8:extensions/markdown.fnl > /tmp/misa-before-source-markdown.fnl
git show d413ee8:extensions/component/markdown.fnl > /tmp/misa-before-source-component.fnl
tools/fennel benchmarks/markdown-source-geometry.fnl \
  /tmp/misa-before-source-markdown.fnl /tmp/misa-before-source-component.fnl
```
