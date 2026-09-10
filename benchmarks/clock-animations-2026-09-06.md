# Clock-driven visual spans, 2026-09-06

Visual spans now compile to fixed-cell text/style patches sampled by the terminal
owner's monotonic clock. Default activity motion no longer dispatches Fennel
timer events. Explicit application animation ticks remain available.

The correctness oracle is `tests/clock-animations.py`: while Fennel is blocked on
a FIFO, native text and color frames must advance without republishing the view
or clearing unrelated rows. The test checks cursor save/restore, inherited styles,
links, zero application animation ticks, silence after static/removal updates,
and exact headless fallback. The previous executable fails waiting for patches;
the candidate passes in Debug and ReleaseSafe. This establishes independence
from session work, rather than a speedup claim.

For a latency regression check, the existing settled-frame workload ran ten
invocations per executable, sequentially with alternating order and three trials
per invocation. Every invocation passed its intermediate-frame and input
correctness assertions. No builds or other tests ran concurrently. Both binaries
use Zig 0.16 ReleaseSafe; CPU affinity was not set. Load average before the run
was 1.19 / 0.97 / 0.77.

| Statistic                                   | Previous commit | Clock animations |
| ------------------------------------------- | --------------: | ---------------: |
| Median of invocation-reported trial medians |        16.20 ms |         16.15 ms |
| Best invocation-reported trial median       |         16.1 ms |          16.1 ms |
| Largest reported trial maximum              |         17.1 ms |          17.0 ms |

The median difference is below 2% and is treated as noise. These invocation
summaries show no meaningful latency regression in this workload; they do not
measure animation throughput or establish a general performance improvement.
The change is retained for independent visual motion and verified span-only
updates without application ticks.

Baseline source: `fde8ad2`. Candidate source: the commit containing this report.
Executable SHA256 values:

- Baseline: `9809bd03220759e157e53517c46fefe3563ce52387ab29e70d7e75f9498e2e11`
- Candidate: `feefa18290741a98e055c2f5d97eb97e6f2fae302a48b2121072f2cc69b329a2`

Raw invocation summaries: `clock-animations-2026-09-06.csv`.

```sh
python3 benchmarks/terminal-threading.py /path/to/baseline /path/to/candidate --samples 10
python3 tests/clock-animations.py /path/to/candidate
```
