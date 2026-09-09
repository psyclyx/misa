# Declarative standard library: regression check

Baseline: `a472cfa`. Candidate: the declarative application/configuration refactor
and namespace/docstring cleanup accompanying this report. Both binaries were
freshly built with `zig build` (Debug), matching the development configuration.
This checks responsiveness; it is not a release-build speedup claim.

The refactor is retained for explicit composition, replaceable named definitions,
and one installation boundary. Input latency and installed startup are comparable.
Source compilation costs more; normal `zig build run` now uses the Lua modules
that its install dependency already builds, instead of compiling them again.

## Method

Use `benchmarks/native-transcript.py zig-out/bin/misa --blocks 300 --rest-ms 25`
in each checkout, with `--mode redraw` or `--mode stream --burst 32 --transport`.
For each mode, alternate baseline/candidate six times with `--samples 1` to check
the output oracle. Then run baseline/candidate/candidate/baseline with
`--samples 10` (20 measured inputs per version per workload). Each process also
warms six frames and verifies the final frame. All repeated and cross-version
frame hashes match within each workload/sample-count combination. Sample counts
produce different stream content, so their hashes are compared separately.

Runs were sequential, without concurrent builds or tests, on an unpinned
interactive desktop with other user applications present. Small timing differences
are not evidence of a speedup. The CSV preserves each process's timing statistics,
startup time, and output hash; it does not contain individual input samples.

## Input latency

Milliseconds; each cell lists the two ten-input process runs. Best is the fastest
input in each process, not the fastest process median.

| Workload | Version | Medians | Best inputs |
| --- | --- | --- | --- |
| Redraw | Baseline | 5.882, 5.962 | 5.204, 5.406 |
| Redraw | Candidate | 5.742, 5.787 | 5.178, 5.445 |
| Streaming | Baseline | 9.886, 9.730 | 9.024, 8.780 |
| Streaming | Candidate | 9.711, 9.441 | 8.843, 8.929 |

Raw process results: [final CSV](stdlib-composition-2026-09-09.csv).

## Startup and rejected eager imports

The initial default specification eagerly required all modules before callers
could remove providers/protocols. This added unnecessary source compilation.
With the same repeated/ABBA protocol, median first-frame times across all eight
processes per version were 707.5 → 821.8 ms (redraw) and 717.4 → 866.6 ms (stream).
That implementation was rejected. The defaults now contain module source names;
only selected modules are required during application construction. A contract
test checks that an excluded source is never loaded.

After that fix, explicit source mode still measures 692.9 → 746.6 ms (redraw)
and 718.9 → 766.0 ms (stream). The source/compiler workload changed with the
expanded declarative definitions and requested public documentation. CPU phase
attribution (`luajit benchmarks/startup.lua`) puts about 1.015 of 1.050 seconds in
Fennel compilation for the full source default; construction plus installation is
about 4.7 ms. That attribution run is diagnostic, not a controlled A/B timing.

The installed comparison avoids source compilation in both versions:

```
python3 benchmarks/installed-startup.py zig-out/bin/misa \
  --baseline /tmp/misa-stdlib-baseline/zig-out/bin/misa
```

Ten launches per version, alternating order, use isolated state/credentials and
each executable's own installed default configuration. Every launch must produce
exactly the expected prompt and no stderr. Baseline median/best: 62.513/60.812 ms;
candidate: 60.852/58.548 ms. Treat this as comparable startup, not a claimed win.
The development run target now uses this same installed module path. Explicit
`MISA_EXTENSION_DIR=.../extensions` still selects the slower source/debug path.

Evidence: [eager-import CSV](stdlib-composition-eager-2026-09-09.csv),
[installed launch samples and binary hashes](stdlib-installed-2026-09-09.txt).
