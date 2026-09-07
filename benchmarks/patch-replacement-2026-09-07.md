# Lazy replacement allocation

Baseline: `24c647c`, `src/lua_runtime/state.fnl`, saved before editing. Candidate
defers allocation of a replacement result until a value differs or a key is
removed. It still traverses and validates every incoming branch, including
equal-identity tables. There is no validation cache or new patch API.

Environment: x86_64 AMD Ryzen 9 5950X, LuaJIT 2.1.1785763465. Commands, with
`BASELINE` denoting the saved baseline Fennel source:

```sh
tools/fennel benchmarks/patch-replacement.fnl BASELINE
tools/fennel benchmarks/transcript-render.fnl BASELINE
```

Both probes warm each implementation six times and run ten alternating A/B,
B/A pairs. The replacement probe compares complete values and unchanged-branch
identity. The transcript probe compares complete rendered JSON records against
one common baseline oracle, retained input state, and final streamed text.
No builds or other agent-run benchmarks ran concurrently.

## Isolated replacement

Each sample makes 64 replacements. Records have an ID, text, and nested map.
The measured operation includes replacement validation and structural sharing.
Timing uses normal GC; allocation is observed separately with collection stopped
for one equivalent sample, then restarted. Values below are milliseconds
median / best, and KiB allocated per 64 replacements, from the second session.

| Records | Replacement | Baseline ms | Candidate ms | Baseline KiB | Candidate KiB |
| --- | --- | --- | --- | --- | --- |
| 1 | Unchanged | 0.040 / 0.039 | 0.026 / 0.024 | 32.5 | 10.0 |
| 1 | Tail changed | 0.041 / 0.039 | 0.040 / 0.037 | 39.5 | 32.5 |
| 1 | All changed | 0.042 / 0.038 | 0.038 / 0.035 | 39.5 | 32.5 |
| 1 | Initial insertion | 0.041 / 0.038 | 0.038 / 0.037 | 52.0 | 51.5 |
| 16 | Unchanged | 0.420 / 0.394 | 0.269 / 0.267 | 294.5 | 10.0 |
| 16 | Tail changed | 0.453 / 0.395 | 0.304 / 0.290 | 301.5 | 39.5 |
| 16 | All changed | 0.424 / 0.400 | 0.407 / 0.388 | 301.5 | 189.5 |
| 16 | Initial insertion | 0.442 / 0.409 | 0.404 / 0.387 | 433.5 | 433.5 |
| 300 | Unchanged | 8.438 / 8.344 | 4.635 / 4.612 | 5371.0 | 10.0 |
| 300 | Tail changed | 8.454 / 8.369 | 4.901 / 4.730 | 5377.5 | 287.5 |
| 300 | All changed | 8.833 / 8.607 | 7.669 / 7.566 | 5377.5 | 3277.5 |
| 300 | Initial insertion | 10.186 / 9.370 | 9.796 / 8.990 | 7781.5 | 7781.5 |

The first session also favored the candidate for 300-record tail replacements:
8.414 / 8.048 ms baseline versus 4.033 / 3.979 ms candidate. The allocation
observation supports the mechanism: unchanged branches no longer create throwaway
result tables. Initial insertion still allocates all new data.

## Whole transcript: no consistent speedup established

Eight-frame combined update-plus-render CPU samples from the second A/B session:

| Blocks | Workload | Baseline median / best ms | Candidate median / best ms |
| --- | --- | --- | --- |
| 1 | Redraw | 0.443 / 0.391 | 0.527 / 0.406 |
| 1 | Stream | 2.318 / 1.719 | 2.420 / 1.767 |
| 16 | Redraw | 7.808 / 7.218 | 7.698 / 7.304 |
| 16 | Stream | 9.065 / 8.694 | 9.112 / 8.679 |
| 300 | Redraw | 191.578 / 182.197 | 193.208 / 187.624 |
| 300 | Stream | 185.641 / 175.359 | 189.010 / 187.440 |

The first A/B session exposed phase-accounting instability: 300-block update
medians were 10.521 ms baseline and 38.311 ms candidate, while rendering medians
were 214.192 and 177.198 ms. In the second session, update medians were 41.101 and
3.982 ms, while rendering medians were 144.325 and 185.116 ms. GC is enabled and
can run in either phase. Therefore the probe now also reports the median and
best of combined per-sample times—not sums of independently computed medians.

The combined result does not establish a whole-transcript improvement, and small
workloads also have worse measured candidate medians. Unchanged redraws do not
call the patch API at all, yet their measurements also differ; they serve as a
negative control against attributing every variant difference to this change.
The change is retained for
its repeatable isolated replacement improvement and large allocation reduction,
not as a streaming-latency win. Native presentation and mixed transcripts remain
outside the probe's scope. Further rendering work must use combined measurements
and must not cite the lower update-phase number alone as an application speedup.
