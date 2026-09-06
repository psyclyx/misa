# Terminal owner separation, 2026-09-06

The session keeps Fennel transactions and semantic projection serialized. A
separate terminal thread receives owned semantic views, prepares native frames,
and presents at the existing 16 ms cadence. Input acquisition, displayed hit maps,
graphics cache, and terminal control commands share that terminal owner.

The correctness benefit is observable independently of timing: the new
`tests/threaded-terminal.py` pauses terminal output, queues a frame, and blocks
Fennel on a FIFO. After output resumes, the queued frame must appear before the
FIFO is released. The serial baseline fails this oracle; the candidate passes.
The test also buffers 128 ordered keys while Fennel is blocked, then checks resize
and terminal restoration at shutdown. Its ten-second deadline detects deadlocks;
the result does not depend on completing Fennel work within a small time budget.

The existing settled-frame PTY workload checks that synchronous dispatch chains
never expose intermediate frames and reports interrupt-to-frame latency during
streaming, including an unthrottled producer. Both executables were built with
Zig 0.16 `-Doptimize=ReleaseSafe` in the development environment. Ten invocations
per executable ran sequentially, alternating baseline/candidate order by round.
Each invocation contains three trials and had to pass its correctness assertions.
No builds or other tests ran concurrently. CPU affinity was not set.

| Statistic | Serial baseline | Terminal thread |
| --- | ---: | ---: |
| Median of invocation-reported trial medians | 16.2 ms | 16.2 ms |
| Best invocation-reported trial median | 16.1 ms | 16.1 ms |
| Largest reported trial maximum | 17.4 ms | 17.1 ms |

These are summaries of three-trial invocations, not raw individual latency
samples. The results support retaining the ownership separation with no observed
median latency regression in this workload; they do not establish a general
speedup, a throughput improvement, or independent Fennel projection. Existing
Fennel animation ticks still use application events.

Baseline source: `c3e879b`. Candidate source: the commit containing this report.
Executable SHA256 values:

- Baseline: `892c5efaf51f5f727ecb905e141d33d2e047b7701d0d16408fb735cb01a38c08`
- Candidate: `9809bd03220759e157e53517c46fefe3563ce52387ab29e70d7e75f9498e2e11`

Raw invocation summaries: `terminal-threading-2026-09-06.csv`.

Reproduce with two separately built executables:

```sh
python3 benchmarks/terminal-threading.py /path/to/baseline /path/to/candidate --samples 10
python3 tests/threaded-terminal.py /path/to/candidate
```
