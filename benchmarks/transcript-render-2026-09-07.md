# Whole-transcript baseline

Source baseline: `122e1ff`. Command:

```sh
tools/fennel benchmarks/transcript-render.fnl
```

Environment: x86_64, AMD Ryzen 9 5950X, LuaJIT 2.1.1785763465.

This runs actual transcript projection, default message/Markdown components,
and theme resolution. Each block contains three short Markdown paragraphs with
bold text, a link, and Unicode. The last block is streaming. Each sample has
eight frames at 80 columns, either unchanged or following eight real transcript
delta reductions and immutable patch applications. Counts include the last block.

The probe records complete JSON output for each frame, then requires identical
output for six unchanged warm-up runs and all ten measured runs. It also checks
retained initial-state contents, final streamed text, and identity preservation
for every unrelated block. Serialization and assertions are outside timing.

All values below are CPU milliseconds **per eight-frame sample**, not per frame.
This is one baseline session, not an A/B comparison or a speedup claim.

| Blocks | Workload | Render median / best | Update median / best |
| --- | --- | --- | --- |
| 1 | Redraw | 0.502 / 0.386 | — |
| 1 | Stream | 2.491 / 1.552 | 0.144 / 0.084 |
| 16 | Redraw | 8.030 / 7.249 | — |
| 16 | Stream | 9.861 / 8.034 | 0.311 / 0.223 |
| 300 | Redraw | 201.924 / 190.665 | — |
| 300 | Stream | 146.440 / 143.248 | 42.365 / 40.997 |

No builds or other agent-run benchmarks ran concurrently. Garbage collection is
enabled: timings charge collection to whichever measured operation triggers it.
The difference between redraw and streaming render timings is not evidence that
streaming makes rendering cheaper. Follow-up attribution should distinguish
allocation/collection costs from model preparation, layout, and theme resolution.

The update path reconstructs the blocks vector and passes it through replacement
materialization, which traverses every replacement branch, including unchanged
blocks. That is a concrete candidate for profiling before redesigning the render
cache. These timings alone do not establish how much of the update cost it owns.

Scope limits: no agent reducer, framework dispatch/fork, syntax highlighting,
selection decoration, attachments, tool components, viewport clipping, native
diff/presentation, terminal I/O, or startup. All blocks belong to one response.
This is a reproducible baseline for a previously unmeasured part of the path,
not completion of representative mixed-transcript or end-to-end performance work.
