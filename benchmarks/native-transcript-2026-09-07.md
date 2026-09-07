# Native transcript frame baseline

Revision: `4dac98f`, ReleaseSafe, baseline CPU. No application code changes.
Run from the repository after `zig build -Doptimize=ReleaseSafe -Dcpu=baseline`:

```sh
python3 benchmarks/native-transcript.py zig-out/bin/misa
python3 benchmarks/native-transcript.py zig-out/bin/misa --rest-ms 25
```

This uses the default extension/UI composition with provider/auth transports
removed, a synthetic transcript, and fixture keyboard events. Each input changes
an editor marker; streaming also dispatches a real transcript delta. The timing
ends when that marker's complete synchronized frame reaches the PTY. This includes
native dispatch, transaction ownership, projections, layout, decoding, presentation,
output and process scheduling, but not a terminal emulator's actual painting.

Every workload has six warm-up frames and ten individual measured frames. Redraw
frames must be byte-identical after normalizing only the sequence marker. Streaming
checks exact accumulated state, unrelated block identity, and current text in the
completed frame; both workloads check the visible OSC link. One final untimed
input validates the last measured delta. Errors/timeouts abort instead of reporting
favorable partial results. Initial insertion is excluded from steady-state timing.

The default terminal driver explicitly paces presentation at 16 ms. Back-to-back
input therefore measured roughly 16–17 ms even at one block. `--rest-ms 25` waits
outside the timed interval so the next input normally arrives after that interval.
This is a different workload, not an optimization or a CPU-time measurement.

Final rested run, wall milliseconds per individual frame:

| Blocks | Workload | Median | Best | Max |
| --- | --- | ---: | ---: | ---: |
| 1 | Redraw | 1.442 | 1.064 | 2.639 |
| 1 | Stream | 2.435 | 1.819 | 4.824 |
| 16 | Redraw | 2.241 | 1.762 | 4.117 |
| 16 | Stream | 4.192 | 3.317 | 6.910 |
| 300 | Redraw | 4.632 | 4.274 | 15.356 |
| 300 | Stream | 10.449 | 8.081 | 21.938 |

Two earlier rested sessions had 300-block streaming medians 9.590 and 10.133 ms,
with maxima 23.404 and 20.937 ms. These runs establish a remaining tail-latency
problem, not its attribution. No builds or other agent benchmarks ran concurrently.
The native path is materially costlier than the standalone Fennel projection probe;
phase instrumentation/profiling must distinguish actual work, allocation/GC and
scheduling before changing production code.

The fixture currently contains assistant Markdown (Unicode, lists, links and
quotes), all in one response. It does not yet represent tool-heavy/multi-response
history, populated cost/syntax records, selection, or image attachments. Its
first-frame column is source-extension startup plus synthetic insertion (roughly
450–550 ms in these runs), **not installed-catalog startup**. Neither this fixture
nor ten measured frames proves every production frame meets the 16.7 ms target.

## Steady-state profiling follow-up

The runner now accepts `--blocks`, `--mode`, and `--samples`. `--perf-output`
attaches `perf record` to the native process after six warm-up frames, then stops
it before app shutdown. Profiled rows are explicitly marked; their timings are
diagnostics, not optimization evidence. Example (with perf in the dev shell):

```sh
python3 benchmarks/native-transcript.py zig-out/bin/misa --blocks 300 --mode stream --samples 200 --rest-ms 25 --perf-output /tmp/misa-frames.data
perf report --stdio --no-children --call-graph none --sort dso -i /tmp/misa-frames.data
tools/fennel benchmarks/transcript-profile.fnl
```

A successful 200-frame native capture attributed 71.41% of sampled user CPU
cycles to libluajit, 23.43% to JIT-generated code, 4.10% to Misa's executable,
and 0.71% to libc. No lost samples were reported. This does not assign waiting
time or determine which functions caused individual latency spikes. A separate
uninstrumented 200-frame run passed all checks: 9.495 ms median, 7.737 ms best,
24.380 ms max. The growing streamed tail wraps; the output oracle counts it
across wrapped lines rather than requiring a single contiguous byte string.

The separate LuaJIT sampler loads the same default extensions and transcript
fixture, settles dispatch chains, and profiles 200 updates after warm-up. It does
not execute native effects other than synchronous dispatch, does not time native
presentation, and uses a fixed clock. It is an attribution aid, not a substitute
for the native oracle. Its latest run collected 2,086 samples; the two largest
individual stacks were recursive `materialize` (359) and `enter!` called from
`materialize` (192). These implicate repeated replacement validation/traversal.
Other prominent stacks involve Unicode clipping and repeated subscription
query-key/lookup work, with GC occurring in those paths. A GC sample identifies
where collection ran, not necessarily where garbage was allocated.

Next experiments should address these Lua-side costs, preserving immutable
validation and rollback contracts. The capture does not justify weakening state
validation or assuming the native renderer is the dominant cost.
