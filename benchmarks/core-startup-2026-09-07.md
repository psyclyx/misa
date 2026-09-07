# Build-translated runtime core startup

Baseline: `f854fe1`; candidate: that commit plus build-time translation of
`state.fnl`, `subscriptions.fnl`, and `framework.fnl`. Both binaries built with
`zig build test -Doptimize=ReleaseSafe -Dcpu=baseline`, Zig 0.16.0 and LuaJIT
2.1.1785763465 on x86_64. Both use the same installed generated extension catalog.

SHA256:

- Baseline: `53dbad2e6c22efb97e9a29f74817a3d221aadb349387c53032278f211e1af4ff`
- Candidate: `164707749ef1381a6672c755e3e1eb1d45ef10ff40f4f4a9f9c2515886060d1b`

Preserve the baseline executable before rebuilding, then run:

```sh
python3 benchmarks/installed-startup.py zig-out/bin/misa --baseline /path/to/saved/baseline
```

The harness alternates A/B and B/A order, ten fresh processes per binary.
Every run must exit zero, emit exactly `misa> enter a prompt:\n`, and emit no
stderr. Credentials and state are isolated, stdin is closed, and child PATH is
empty to prevent provider CLI execution. All twenty runs passed this oracle.
No builds or other agent workloads ran concurrently. CPU affinity and OS caches
were not controlled. Load average immediately before measurement was about 1.6.

| Binary | Median wall ms | Best wall ms |
| --- | ---: | ---: |
| Baseline | 81.000 | 78.390 |
| Translated core | 33.557 | 31.409 |

This is a 58.6% median reduction in process startup through EOF shutdown, **not**
interactive first-frame latency or authenticated-provider readiness. The compiler
itself still loads for custom Fennel extensions. The change removes compilation
of the invariant core, not application setup or native initialization.

Raw wall seconds, paired by iteration (execution order alternates):

```text
baseline       candidate
0.081242488    0.033491420
0.084286464    0.033234916
0.078446954    0.031627161
0.079099045    0.034437345
0.078389503    0.033838685
0.080914782    0.033926577
0.083426591    0.034471425
0.081084315    0.033622912
0.080725199    0.033305667
0.085878588    0.031408778
```

Before edits, the existing source/installed harness independently reproduced an
installed median of 78.226 ms (best 74.477 ms, ten runs). The interleaved binary
comparison above is the basis of the claimed improvement.

The complete release/baseline test suite passed, including native bootstrap
stack cleanup, correlated core source diagnostics, custom Fennel loading and
failure recovery, and installed generated catalog integration tests. Core output
is portable Lua source embedded through declared build inputs, not LuaJIT
bytecode. A fresh Nix package build was not run for this slice.
