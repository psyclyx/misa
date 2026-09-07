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
