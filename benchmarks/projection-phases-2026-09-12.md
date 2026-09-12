# Transcript projection phases

Revision: the worktree at the review follow-up, Debug build, `.zig-cache` isolated
from any live session. No application code changes.

```sh
tools/fennel benchmarks/projection-phases.fnl 300 60
```

This harness loads the default composition with the `benchmarks/native-transcript`
fixture, dispatches real frames through the transaction pipeline, and times two
phases per frame with native effects discarded: model dispatch plus commit, and
view projection plus commit. It reports the best per-frame time over the
requested iterations, which removes most garbage-collection noise from a
single-threaded LuaJIT run. It does not time native presentation, terminal
output, or scheduling; `benchmarks/native-transcript.py` covers those.

Three frame kinds isolate where the cost of a transcript comes from:

- `static` projects committed state again without dispatching, so every
  subscription and projection cache is a hit.
- `redraw` dispatches a frame that changes the editor but not the transcript,
  which is what an ordinary keystroke costs.
- `delta` dispatches one streamed transcript delta, which changes the block
  array.

Best milliseconds per frame, 60 iterations, mixed scenario:

| Blocks | Frame  | Model dispatch | Presentation |  Total |
| -----: | ------ | -------------: | -----------: | -----: |
|      1 | static |              – |       0.2190 | 0.2190 |
|      1 | redraw |         0.0060 |       0.2400 | 0.2460 |
|      1 | delta  |         0.0670 |       0.3930 | 0.4600 |
|     16 | static |              – |       0.3630 | 0.3630 |
|     16 | redraw |         0.0050 |       0.3810 | 0.3860 |
|     16 | delta  |         0.0640 |       0.5560 | 0.6200 |
|    300 | static |              – |       0.3770 | 0.3770 |
|    300 | redraw |         0.0090 |       0.4300 | 0.4390 |
|    300 | delta  |         0.1000 |       1.0490 | 1.1490 |
|   1000 | static |              – |       0.3610 | 0.3610 |
|   1000 | redraw |         0.0060 |       0.3730 | 0.3790 |
|   1000 | delta  |         0.1400 |       2.2990 | 2.4580 |
|   3000 | static |              – |       0.5470 | 0.5470 |
|   3000 | redraw |         0.0090 |       0.5080 | 0.5210 |
|   3000 | delta  |         0.3750 |       7.9680 | 8.3430 |

Repeated runs at 1000 and 3000 blocks moved the delta presentation column by
under 6%, and the redraw column by under 0.05 ms; the table above is one run of
each size.

## What the numbers say

A redraw frame is flat in transcript size: 0.24 ms of presentation at one block
and 0.51 ms at 3000. `misa.transcript.project` declares the whole block array as
an invalidation input, so a frame that does not replace it reuses the accepted
projection and never re-renders the transcript. The `static` row shows the same
thing from the other side: projecting committed state again costs the root
composition, not the transcript.

A delta frame is not flat. It replaces the block array, which invalidates the
whole projection, so every frame renders the transcript again through the
per-item component cache. That cache keeps the _rendered geometry_ of unchanged
items (only the streamed item re-renders, which the frame oracle confirms), but
the walk still builds a presentation model for every block, consults the
selection and syntax services per block, and assembles one line array for the
whole transcript. The cost is linear: about 2.7 µs per block per delta, so 1.05 ms
at 300 blocks, 2.3 ms at 1000, and 8.0 ms at 3000.

Model dispatch is cheap by comparison (0.38 ms at 3000 blocks) and is not the
target here.

## Scope

These are Lua-side phase timings, not frame latency. In the same session the
native frame benchmark measured 5.4 ms for a 300-block redraw and 9.0 ms for a
300-block delta, so the delta projection above accounts for roughly a tenth of a
300-block delta frame; native frame construction, terminal output, and the
streamed item's parse and layout make up the rest. The harness cannot attribute
those, and it does not model a real terminal.

The measurements say nothing about transcripts larger than 3000 blocks, other
frame kinds (resize, hover, scroll), or non-interactive rendering.
