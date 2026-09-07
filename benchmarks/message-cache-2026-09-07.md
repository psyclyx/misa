# Rejected per-message subscription cache

Baseline: `ff08c14`. The prototype replaced the global message projection map
with `[:message/body response-id block-id]` subscriptions. Seven explicit leaf
dependencies described each body's immutable input record. A third compute
argument supplied a disposable previous result for incremental Markdown work.
Functional tests passed, including rejected-render cache isolation.

**Rejected:** this design substantially regresses redraws and thrashes once the
working set exceeds the shared scope's 256-entry bound. Both the component
cutover and the evaluator API addition were reverted. The complete rejected
diff is preserved in `message-subscriptions-rejected.patch`, not active code.

Reproduce from `ff08c14` (or a compatible tree): save
`extensions/component/message.fnl` outside the tree, apply the archived patch,
then run:

```sh
tools/fennel benchmarks/message-cache.fnl /path/to/saved/message.fnl
```

LuaJIT 2.1.1785763465, bundled Fennel, x86_64. Each message contains twenty
paragraphs with bold text and a link. Samples measure CPU time for three full
raw message-render passes at 80 columns, excluding JSON output comparison.
Six warm-up passes per implementation precede ten alternating A/B, B/A samples.
Every render's semantic output matched the baseline oracle, including warm-ups.
No builds or other agent workloads ran concurrently. Affinity and OS caches
were not controlled. This excludes theme resolution, native output, and dispatch.

| Messages | Baseline median/best ms | Prototype median/best ms |
| ---: | ---: | ---: |
| 1 | 0.063 / 0.053 | 0.132 / 0.101 |
| 16 | 1.334 / 0.874 | 1.935 / 1.441 |
| 300 | 34.399 / 28.524 | 1075.684 / 1061.682 |

Raw CPU milliseconds for the discriminating 300-message workload:

```text
baseline prototype
35.353   1075.850
28.829   1076.023
31.274   1062.124
35.007   1075.518
35.137   1067.207
35.776   1081.213
34.729   1061.682
28.524   1080.453
31.551   1064.939
34.069   1076.743
```

The mechanism inferred from code and the working-set cliff is query eviction:
300 body queries cannot coexist in a 256-entry scope, even before leaf queries
and other consumers. Visiting every message on every redraw continuously evicts
results needed on the next pass. The smaller workloads also expose query
evaluation overhead; this is not solely a capacity problem.

Next design constraint: own the transcript's derived body collection as a
collection, retaining structural sharing inside it. Do not make every historical
message compete with unrelated application queries for a flat cache slot, and do
not raise an arbitrary limit to hide the cliff. Keep the existing committed pure
Markdown projection API; it permits explicit reuse within that collection.
The previous-result evaluator extension is not accepted until a useful consumer
and its bounded ownership model are verified together.
