# Default model picker: held-arrow latency, 2026-09-07

The final implementation removes the demonstrated navigation backlog without
dropping, coalescing, or reordering input. At 3,000 models and 60 presses/second,
the median time from the last injected press to the completed frame containing
the exact final focused row fell from **3,046.7 ms to 30.9 ms**. This is PTY
completed-frame latency, excluding the terminal emulator's painting and physical
keyboard/key-up delivery. A final row can still arrive one or two frames after
the last press; the test does not claim zero input latency.

## Workload and oracle

`picker-key-repeat.fnl` registers 1,000 or 3,000 synthetic models with real model
IDs, labels, provider, and context-window metadata. It opens the same entry point
as default Alt-M: `model/picker-open` → `choices/command-open` → the actual
`/model` picker. The full default UI, model completion, Browse/Favorites panels,
preview, key routing, subscriptions, native decoder, transaction validation,
terminal actor, and presentation run normally. Only provider/auth I/O is omitted.
The fixture does not replace the picker renderer or insert a measurement marker.

Python sends arrows at 30 or 60 Hz for one second. It parses the actual `>`
focused model row inside complete synchronized terminal updates. Each hold must
reach the exact row implied by every supplied key; observed focused rows must
move monotonically without overshoot. Intermediate frames may be superseded by
the existing presenter. Final frame SHA256 values matched byte-for-byte across
variants and repeated holds. All injected key sequences passed the row oracle.
Maximum observed send-scheduler lateness was 0.210 ms.

Each size ran baseline/candidate/candidate/baseline invocation blocks. Each
invocation included six warm-up holds and five measured holds at each rate,
yielding **10 measured holds per variant/rate/size**, 80 measured holds total.
Directions alternate; every invocation uses the same sequence. No builds,
tests, or other agent benchmarks ran during measurement. CPU affinity was not
pinned; normal GC remained enabled.

## Results

Wall milliseconds from the final input write to receipt of the completed frame
showing its final row. These are per-hold latencies, not CPU time per key.

| Models | Presses/s | Baseline median / best / max | Final median / best / max | Max pending rows, before → after |
| --- | ---: | ---: | ---: | ---: |
| 1,000 | 30 | 23.006 / 20.485 / 34.407 | 8.524 / 7.627 / 13.307 | 2 → 1 |
| 1,000 | 60 | 353.158 / 266.535 / 412.575 | 18.593 / 10.894 / 25.664 | 18 → 2 |
| 3,000 | 30 | 1,067.914 / 934.090 / 1,214.527 | 19.014 / 16.361 / 22.806 | 17 → 1 |
| 3,000 | 60 | 3,046.735 / 2,840.800 / 3,274.053 | 30.948 / 20.463 / 36.367 | 47 → 3 |

Raw summaries and final-frame hashes: `picker-key-repeat-2026-09-07.csv`.
Raw injected-key and observed-focus timestamps, including warm-ups and resets:
`picker-key-repeat-2026-09-07.json.gz`.

## Mechanism and correctness

LuaJIT sampling of the real picker transaction/default UI found repeated full
catalogue materialization, browse construction, and later deep comparison.
The earlier service-only optimization did not expose these end-to-end costs.

- Choice movement constructs small immutable focus records; picker updates
  emit changed session fields instead of repeatedly replacing the entire picker.
  Incoming changed state still passes the ordinary patch validator.
- Built-in item projection belongs to the existing transactional subscription
  scope. Its explicit inputs cover source items, query, preference scope/data,
  view ID, config, and the previous committed item array. The last input allows
  adoption of canonical arrays after validation, then stable reuse while focus
  moves. There is no global memo table or state-validation exemption.
- Custom registered views still project against the complete current database
  and session. Built-in IDs cannot be replaced. Tests verify favorite/usage
  invalidation, return to previous preferences, unrelated-state identity, and
  custom views reading an unrelated database field. The existing subscription
  transaction test verifies rejected transactions preserve committed caches.
- Ordinary navigation skips pre-input geometry construction. Visible shortcut
  and pending-sequence paths still resolve their current targets. The same skip
  applies to inline choices, but generic dynamic completion semantics are retained.

An intermediate equality-only refresh plus changed-field updates still left
about half a second of backlog at 3,000/60 Hz. That result motivated moving
invariant item projection to its declared dependency owner. No intermediate
number is presented as the final result.

## Build and baseline definition

AMD Ryzen 9 5950X, x86_64 Linux 7.0.14-zen1, LuaJIT 2.1.1785763465, Zig 0.16.0.
Both variants used **the same Debug executable**, matching the plain `zig build
test` development build in which the issue was reproduced. This deliberately
measures the actual development configuration; it is not a ReleaseSafe or
production-throughput claim. Executable SHA256:
`c8d8bc7d2c11d171d046b0c898738a82cdc6363e3d03367e7df1adf7b8f57642`.

The baseline is the performance implementation at the start of the user's
“keep going” follow-up, **not original repository HEAD**. It already includes the
prior service-only duplicate-refresh removal. A performance-only reverse patch
preserves the final unrelated picker UX while restoring that implementation.
Each variant loaded a saved extension tree via `MISA_EXTENSION_DIR`; the same
executable, fixture, configuration, catalogue, and input schedule were used.

Reconstruct a baseline tree by copying current `extensions/` and applying
`picker-key-repeat-baseline.patch` inside that copy with `patch -p1`. Then run:

```sh
python3 benchmarks/picker-key-repeat-compare.py /path/to/misa /path/to/baseline extensions > results.csv
```

For one direct workload or raw timestamps:

```sh
python3 benchmarks/picker-key-repeat.py /path/to/misa --models 3000 --rates 60 --trace trace.json
```

The timing result covers the default model overlay. Inline command completion,
provider metadata arrival during a hold, different terminal dimensions, and
terminal-emulator paint timing are not benchmarked here. The generic inline
completion source still refreshes on consumed input; no query-only shortcut was
introduced that would hide database-dependent source changes.
