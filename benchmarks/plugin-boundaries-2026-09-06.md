# Plugin boundaries, 2026-09-06

## Rejected blanket execution guard

The pinned shell uses LuaJIT `2.1.1785763465` (Nix store package
`x53la7l94pyqlwmq7mdp740v70ahpj8j-luajit-2.1.1785763465`). A mechanism probe warmed
this function with 100 calls of 1,000 iterations, then called it with 100,000,000:

```lua
local function hot(n)
  local x = 0
  for i = 1, n do x = x + i end
  return x
end
for i = 1, 100 do hot(1000) end
-- Insert the selected mode change here.
local ticks = 0
debug.sethook(function()
  ticks = ticks + 1
  if ticks > 10 then error("budget") end
end, "", 1000)
local ok, result = pcall(hot, 100000000)
debug.sethook()
print(ok, result, ticks)
```

| Mode change after warm-up | Observed outcome | Hook calls |
| --- | --- | ---: |
| None | Success, sum `5.00000005e+15` | 0 |
| `jit.off()` | Success, same sum | 0 |
| `jit.off(); jit.flush()` | Error `budget` | 11 |

This is a correctness probe, not a timing benchmark. A count hook alone does not
bound already compiled hot loops. Disabling compilation alone leaves those traces
executable. LuaJIT's [C API documentation](https://luajit.org/ext_c_api.html) makes
the engine-off and cache-flush controls separate; its
[dispatch implementation](https://github.com/LuaJIT/LuaJIT/blob/v2.1/src/lj_dispatch.c)
shows the distinct branches. Disabling a registered function and its lexical
children also does not cover arbitrary captured or imported helper functions.

We did not implement a blanket interpreter-mode guard. It would change the normal
execution path to address an exceptional case, and no performance evidence here
justifies that tradeoff. A Lua hook would still not preempt a blocking native
callback. Any future guard must separately specify recovery of framework
transaction state, hook-error handling, callback attribution, and native blocking
limits.

## Ownership simplification

Terminal presentation already has one owner and consumes prepared animation and
graphics plans. It now also adopts prepared frame bytes and action maps after
successful output. This removes the extra frame-buffer copy and deep action-map
clone while preserving the previous displayed state on presentation failure.
The existing shown-frame action test checks pointer transfer and suspended-frame
ownership, alongside semantic behavior. No timing improvement is claimed by this
change alone.

Theme resolution constructs output records without mutating cached semantic
spans. Message wrappers share those spans, and zero-height transcript viewports
skip projection. Syntax requests run as native effects; the renderer consumes
immutable derived captures and documents through a resolver that survives the
component input snapshot without deep-copying their arrays. Only request and
accepted-revision metadata enters transactional state. Capture history is pruned
against committed revisions, and unchanged blocks retain their layout cache.

## Responsiveness oracles

`tests/dispatch-limit.py` gives a plugin an endless chain of promptly returning
handlers. The previous executable fails its ten-second recovery deadline; the
candidate retains the seven committed updates allowed by the test configuration,
reports the limit, handles another key, repeats recovery, and quits normally.
Headless overflow exits explicitly. This bounds chains, not individual callbacks.

`tests/syntax-worker.py` holds grammar loading on a FIFO and checks that a keyboard
transaction renders before releasing the loader. The candidate passes; releasing
an invalid grammar produces empty captures and normal shutdown. This oracle
uses the new effect API and is not a comparable speed benchmark for the previous
synchronous API. Native tests separately force zero available worker capacity and
verify failure completion without inline execution. Parser cancellation/reset,
streaming coalescing, stale completions, and cache rollback have dedicated tests.

## Settled-frame latency check

The existing three-trial settled-frame workload ran ten invocations per binary,
sequentially with alternating order. Every invocation passed correctness checks.
No builds, agent work, or other tests ran concurrently; CPU affinity was not set.
Both executables were ReleaseSafe builds using the pinned Zig 0.16 toolchain.

| Statistic | Before | Candidate |
| --- | ---: | ---: |
| Median of invocation-reported trial medians | 16.2 ms | 16.1 ms |
| Best invocation-reported trial median | 16.1 ms | 16.1 ms |
| Largest reported trial maximum | 17.1 ms | 17.1 ms |

The difference is below 2% and treated as noise. This workload shows no meaningful
latency regression; it does not measure syntax throughput or prove a general
speedup. The changes are retained for explicit ownership, removal of blocking
native work from projection, and demonstrated dispatch-loop recovery.

Baseline source: `1a4ea64`. Candidate includes `b96ea32` and the commit containing
this report. Executable SHA256 values:

- Baseline: `feefa18290741a98e055c2f5d97eb97e6f2fae302a48b2121072f2cc69b329a2`
- Candidate: `61cec3786e93195fca0052f99f24955c0a33fb2ca543fac38f91545cefabea99`

Raw invocation summaries: `plugin-boundaries-2026-09-06.csv`.

```sh
python3 benchmarks/terminal-threading.py /path/to/baseline /path/to/candidate --samples 10
python3 tests/dispatch-limit.py /path/to/candidate
python3 tests/syntax-worker.py /path/to/candidate
```
