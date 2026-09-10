# Final architecture handoff checks

Source: `0c14a02`. ReleaseSafe, baseline CPU build via
`zig build -Doptimize=ReleaseSafe -Dcpu=baseline`.
Binary SHA256: `6790e8c3d9fa0760437a568d523dc8de9def6c3270ffc850f01e0c63ee1e91f9`.
Host load before measurement: 0.56 / 0.93 / 0.90. No concurrent builds or agent
workloads were run during these measurements. CPU affinity and OS caches were
not controlled.

This is a bounded final check using existing harnesses, not a new optimization
campaign. No renderer machinery was added to meet a frame deadline.

## Startup

Run `python3 benchmarks/installed-startup.py zig-out/bin/misa`.
Ten fresh processes per mode, alternating source/installed order, same binary.
Every process exited zero, emitted exactly `misa> enter a prompt:\n`, and had
empty stderr. Credentials/state were isolated and child PATH was empty.

| Catalog                 | Median wall ms | Best wall ms |
| ----------------------- | -------------: | -----------: |
| Source Fennel           |        588.329 |      571.594 |
| Installed generated Lua |         44.032 |       43.439 |

These are process-start-through-EOF-shutdown numbers, not interactive readiness
or authenticated-provider readiness. Build-time extension translation remains
effective on the final profile. The prior 33.557 ms installed result belongs to
an earlier binary/profile, not the current one; this check is not an interleaved
comparison against that binary. Earlier attribution and core-translation A/B
evidence remain in `core-startup-2026-09-07.md`.

Raw wall seconds:

```text
source:
0.6099204220081447 0.5748464089992922 0.5715935799962608 0.586388513998827
0.5917204260040307 0.5895480930048507 0.5876755439967383 0.591307319002226
0.5889818639989244 0.5842660420021275
installed:
0.04693920200224966 0.0436944230023073 0.04433817300014198 0.04443665400322061
0.0438622950023273 0.04367599300167058 0.04385779499716591 0.04420143099559937
0.043439069006126374 0.04493251100939233
```

## Mixed transcript

Run `python3 benchmarks/native-transcript.py zig-out/bin/misa --blocks 300
--scenario mixed --rest-ms 25 --samples 10` (one command).
The fixture contains multiple user/assistant responses, thinking, tool calls,
code/syntax, and response metadata. Each workload uses warm-up frames before ten
measured inputs. The 25 ms untimed gap avoids deliberately colliding with the
presenter's 16 ms pacing interval. No provider/network requests are made.

| Workload     | Median ms | Best ms | Maximum ms |
| ------------ | --------: | ------: | ---------: |
| Redraw       |     2.426 |   2.201 |      2.859 |
| Stream delta |     8.115 |   6.991 |     19.983 |

All synchronized-frame marker, stream-content, unchanged-block identity, and
frame-hash assertions passed. Normalized frame SHA256:

- Redraw: `b2c88ad8c309118e00f1e5c1e1f7e1ab78ed61d2ac7a342c4fe23c549d63946d`
- Stream: `bc370ba0a9a4a4e9585a41e51364a0d19affb165d8f78f2b455e22c2a6c8c7dc`

This is input-to-complete-PTY-frame wall time, including application work and
output scheduling, excluding emulator painting. Synchronized-output markers
delimit coherent frames, not paint acknowledgements. This small sample is not a
tail-latency estimate or a guarantee that every frame fits 16 ms. Fixture first
frames took 952.507 / 932.229 ms, including loading/seeding the benchmark and
settling syntax; these are not default interactive startup measurements.

Selective recomputation, rollback, raw metadata, and hover preservation have
separate executable regressions (`component-projections`, `indicator-values`,
`transcript-delta-state`, `response-metadata`, and installed PTY interaction tests).
The renderer still performs general transcript traversal and viewport assembly;
no universal O(1) streaming or every-workload frame-rate claim is made.
