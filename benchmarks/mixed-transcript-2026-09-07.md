# Mixed native transcript baseline

Runtime: `bbae7c6`, ReleaseSafe, baseline CPU. This adds a workload and verification
coverage; it is not a production optimization or an A/B speedup claim.

```sh
python3 benchmarks/native-transcript.py zig-out/bin/misa --scenario mixed --rest-ms 25
python3 benchmarks/native-transcript.py zig-out/bin/misa --scenario mixed --blocks 300 --mode stream --samples 1000 --rest-ms 25
python3 benchmarks/native-transcript.py zig-out/bin/misa --scenario mixed --blocks 300 --mode stream --samples 1000 --rest-ms 0
```

## Fixture and oracle

The new `mixed` scenario repeats a user block followed by an assistant response
containing a plan/thinking block, a completed tool call, and a code-fenced
Markdown reply. Tool calls have structured arguments and diff-like result text;
verbose mode exposes their full previews. The last reply streams real transcript
deltas. At 300 blocks there are 150 responses, 75 tool calls and 75 code-fenced
replies. Only the 74 completed assistant replies carry response metadata,
matching the app's response-end behavior; user turns and the active reply do not.
The one-block case contains only the streaming reply.

The fixture validates response indices, contiguous ownership ranges and total
block coverage and metadata ownership before timing. It issues actual native highlighting requests and
withholds the initial frame marker until all expected documents have nonempty
capture data and no pending requests. Successful captures, not merely request
completion, are required. This also keeps asynchronous startup ordering out of
the frame-output oracle.

Each measured update preserves unrelated block and response identities and must
retain every streamed byte. The native reader checks complete synchronized
frames, visible code and tool output in the initial warm-up frame, and OSC links.
Redraw output is identical after marker normalization. Ten independent runs
produced one complete frame hash per workload. All six default `markdown`
scenario hashes still match the pre-change oracle.

Only input generation and initial state are fixtures. Provider/auth transports
are excluded; default components, syntax processing, native decoding, viewport
layout and terminal presentation are exercised. The first-frame column includes
source-extension loading, fixture insertion and initial highlighting—about
0.9 seconds for this 300-block fixture—not normal installed-app startup.

## Rested baseline

Ten runs, each with six warm-ups and ten measured frames per workload. No builds,
tests, profiles or other agent-run benchmarks ran concurrently. Wall milliseconds
per frame; median / best are **of the ten run medians**. Maximum is the largest
individual measured frame across those runs.

| Blocks | Workload | Median / best | Maximum |
| --- | --- | ---: | ---: |
| 1 | Redraw | 1.271 / 1.109 | 2.958 |
| 1 | Stream | 2.120 / 1.932 | 6.190 |
| 16 | Redraw | 1.545 / 1.378 | 3.807 |
| 16 | Stream | 2.754 / 2.589 | 5.987 |
| 300 | Redraw | 2.541 / 2.278 | 12.093 |
| 300 | Stream | 4.960 / 4.448 | 18.329 |

## Longer observations and pacing

Separate 1,000-frame streaming runs at 300 blocks produced matching frame hashes
with and without the idle gap:

| Idle gap | Median | Best individual frame | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| 25 ms | 4.712 | 3.646 | 14.958 | 17.353 | 19.681 |
| 0 ms | 17.019 | 16.270 | 17.335 | 17.378 | 17.873 |

The driver enforces a 16 ms presentation interval (`src/terminal/driver.zig`).
With no idle gap, the next input follows the previous completed frame immediately,
so measured latency includes presenter pacing; it is not 17.019 ms of rendering
CPU time. Neither run dropped stream data. This closed-loop producer does not
test an independently scheduled input stream or input backlog/coalescing.

The synchronized-output markers delimit a complete frame received at the PTY;
they do not acknowledge terminal painting. These timings include input handling,
projection, native presentation and OS/pipe overhead, not GUI paint or display
refresh. Component computation can be timed internally without adding terminal
flushes or changing the renderer's synchronization boundaries.

The mixed case still exceeds the 16.7 ms budget in its tail. It broadens coverage
beyond the single-response benchmark but does not close the every-frame target.
Images, active selection, streaming tool arguments and populated cost ledgers
remain outside this workload.

One next candidate exposed by this shape is response-cost lookup: source
inspection shows a collection subscription read for each metadata-bearing
response (74 here). That traversal has not yet been isolated or timed; it is a
profiling/experiment lead, not an attributed performance result.

Verification: the ReleaseSafe/baseline full test suite passed. The ten corrected
baseline runs and both long observations passed all native fixture assertions.
An initial fixture incorrectly attached metadata to every response; the table
above uses only measurements rerun after correcting that mismatch.
