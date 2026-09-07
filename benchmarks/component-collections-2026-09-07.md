# Collection-owned rendering

Baseline source: `1ac6815`, saved `extensions/messages.fnl` and
`extensions/component/message.fnl` before the cutover. Candidate replaces the
message component's global document cache with explicit immutable hints, and
retains semantic output and themed views in a collection subscription.

```sh
tools/fennel benchmarks/transcript-render.fnl --projection-baseline BASELINE_DIRECTORY
```

`BASELINE_DIRECTORY` contains those files as `messages.fnl` and `message.fnl`.
The probe installs the saved message components under separate baseline role IDs
and calls the saved transcript projection for baseline samples. Both variants
use the same current patch implementation and theme resolver. Complete rendered
JSON for every frame must match a common baseline oracle. Setup and oracle
serialization are not timed.

Environment: AMD Ryzen 9 5950X, x86_64, LuaJIT 2.1.1785763465. Six warm-ups per
variant; ten alternating A/B, B/A sample pairs. No builds or other agent-run
benchmarks ran concurrently. Every sample contains eight frames at 80 columns.

## Results

CPU milliseconds **per frame**, calculated by dividing each eight-frame sample
by eight. These are sample-average frame costs, not individual-frame percentiles.
Streaming includes transcript reduction/patch application plus rendering.

| Blocks | Workload | Baseline median / best | Candidate median / best |
| --- | --- | --- | --- |
| 1 | Redraw | 0.062 / 0.052 | 0.015 / 0.013 |
| 1 | Stream | 0.293 / 0.205 | 0.315 / 0.223 |
| 16 | Redraw | 1.027 / 0.868 | 0.055 / 0.033 |
| 16 | Stream | 1.346 / 1.050 | 0.338 / 0.254 |
| 300 | Redraw | 28.741 / 24.523 | 2.631 / 0.646 |
| 300 | Stream | 25.071 / 21.866 | 1.791 / 1.149 |

The one-block streaming case adds about 0.02 ms per frame in this session. The
collection/dependency machinery has overhead when there is no unchanged history
to reuse; this tradeoff is recorded rather than presenting every workload as a
win. The substantial retained-history improvement is the reason for the cutover.

GC remains enabled. In the 300-block streaming case, update-phase medians alone
were 3.397 ms baseline and 7.161 ms candidate **per eight frames**. Combined sample
medians were 200.567 and 14.331 ms. Use combined timing, not a selected phase, when
comparing streaming work. Interleaving may charge collection of one variant's
garbage to the other; the conservative measured result still strongly favors
the collection on retained-history workloads.

## Mechanism and correctness

- One subscription entry owns a collection, not one entry per message. Immutable
  per-item entries retain semantic output, optional incremental hints, resolved
  output, and the relevant action/link targets.
- Equal model/context fields and implementation identity reuse semantic output.
  Theme changes and relevant hover transitions resolve that output again without
  invoking the semantic component. Unaffected resolved views retain identity.
- Default Markdown projection hints are passed explicitly. No global message
  cache or reset handler remains, so rejected transactions cannot publish caches.
- Syntax and cost enrichment also use incremental collection projections. A
  300-item regression test checks that these lookups do not evict the render
  collection through the bounded flat query index.
- Tests cover changed/removed items, layout changes, component/theme switches,
  action/link precedence, duplicate IDs, and native-rejection/Lua-failure rollback.

The timing fixture still has short assistant Markdown blocks in a single
response, without syntax/cost enrichment. The enrichment behavior is tested, not
timed here. Native dispatch/fork, selection, viewport clipping, tool components,
attachments, terminal diff/I/O, and startup remain outside this benchmark.
Model preparation and collection assembly still walk the transcript. This is
not a claim that every production frame now meets a particular latency bound.
