# Build-time compilation experiment

Run `python3 benchmarks/startup-precompiled.py` from the repository. Source under
test is `fb9d911` plus this experiment. LuaJIT `2.1.1785763465`, x86_64;
Python `3.13.12` orchestrated the runs. The tool translates every bundled extension
and the three runtime Fennel modules into ordinary Lua in a fresh temporary
directory. No generated files are installed or checked in. Generation occurs
before timing; temporary artifacts are removed after the experiment.

Both modes use `benchmarks/startup.lua`, including compiler bootstrap. Source mode
compiles at runtime; generated mode reads the translated files. Ten fresh-process
samples per mode ran sequentially in alternating order (A/B, B/A), with no builds
or other agent workloads in parallel. CPU affinity and OS cache state were not
controlled. This is the isolated default-profile setup/first-dispatch probe,
**not installed-app latency**; native effects are never executed.

All 20 samples produced byte-identical semantic output, SHA256
`c1a1d5ebcbffd618ba271af75c7da75f9f9014958649a70bd76d0ebe0d3a3d5e`.

| Measurement | Source median / best ms | Generated median / best ms |
| --- | ---: | ---: |
| Compilation, CPU | 538.451 / 509.688 | 0 / 0 |
| Lua loading/evaluation, CPU | 9.259 / 8.738 | 8.272 / 7.851 |
| Setup, CPU | 1.500 / 1.095 | 1.311 / 1.236 |
| First dispatch/commit, CPU | 1.687 / 1.350 | 2.009 / 1.761 |
| Total probe, CPU | 557.533 / 529.142 | 18.091 / 17.110 |
| Subprocess wall | 565.200 / 541.146 | 20.757 / 19.501 |

The first dispatch is slightly slower in generated mode; this probe does not
attribute that difference or establish steady-state streaming performance. The
overall reduction is dominated by removing compilation from the timed path.

Raw wall seconds, sample order within each mode:

```text
source      generated
0.558836898 0.021466537
0.552496972 0.020372000
0.575065825 0.021204503
0.566118849 0.021107351
0.549542296 0.020945199
0.589223530 0.020198538
0.541146068 0.019990004
0.568593657 0.020979740
0.568370592 0.020568733
0.564280660 0.019500987
```

## Integration requirements

The runtime loading path is unchanged by this experiment. Production integration
must generate artifacts with build dependencies on both compiler and source;
use host LuaJIT for source translation (not target-specific bytecode); retain
custom `.fnl` loading and compiler availability; preserve source diagnostics;
and distinguish installed catalog artifacts from explicit source-directory
overrides. The current resolver returns `.fnl` paths for both cases, so blindly
preferring adjacent generated files would risk stale development overrides.
Installed binaries and package builds still need parity and latency verification.
