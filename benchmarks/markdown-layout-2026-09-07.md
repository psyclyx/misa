# Immutable incremental layout probe

Baseline: `569af90`'s `extensions/component/markdown.fnl`; candidate: explicit
immutable projection API. LuaJIT 2.1.1785763465, bundled Fennel compiler, x86_64.
Save the baseline source before editing, then run:

```sh
tools/fennel benchmarks/markdown-layout.fnl /path/to/baseline.fnl
```

The probe uses forty repeated heading/paragraph/code sections, appended in
128-byte chunks, plus a separate workload of 100 identical full-source redraws.
It compares JSON-encoded semantic lines for every output. Six baseline runs
establish deterministic output; both implementations receive six warm-up runs.
Ten measured runs per implementation alternate A/B and B/A. Timers enclose only
layout calls, not compilation or output comparison. Each sample is cumulative
CPU time for an entire sequence, not per frame. No agent build/test ran alongside
measurement. Affinity and OS caches were not controlled.

| Workload | Baseline median/best ms | Candidate median/best ms |
| --- | ---: | ---: |
| Streaming sequence | 2.997 / 2.819 | 2.721 / 2.581 |
| Initial layout + 99 redraws | 1.785 / 1.735 | 1.691 / 1.654 |

All outputs matched. These small, single-session timings support retaining the
ownership change, not an end-to-end UI speed claim. There is no native terminal,
theme resolution, transaction work, or provider stream in this probe. The new
projection uses immutable positional block entries, eliminating a mutable weak
map and allowing unchanged input to return its prior projection directly.

Raw CPU milliseconds, paired by sample (execution order alternates):

```text
stream baseline candidate     redraw baseline candidate
       3.381    2.598                 1.793    1.690
       2.892    2.581                 1.769    1.654
       2.819    2.676                 1.786    1.673
       2.995    2.785                 1.775    1.692
       3.576    2.739                 1.827    1.702
       2.947    2.680                 1.793    1.699
       2.953    2.840                 1.735    1.687
       3.528    2.703                 1.784    1.703
       2.998    2.760                 1.814    1.705
       3.297    2.934                 1.758    1.686
```

An initial probe warmed only the baseline while checking determinism. Its
streaming medians were 3.187/3.114 ms and redraw medians 1.569/1.618 ms
(baseline/candidate). That asymmetric warm-up was corrected before the measured
comparison above; those initial numbers are not evidence of a performance win.
