# Historical first-stage service projection measurement, 2026-09-07

This is an intermediate measurement from before the held-key PTY investigation.
Its candidate is **not the final implementation**. The final changes, actual
release-to-visible-row measurements, and working baseline reconstruction are in
[`picker-key-repeat-2026-09-07.md`](picker-key-repeat-2026-09-07.md).
The CSV below remains historical evidence for that first, smaller change.

Navigation rebuilt the browse catalogue twice during each input transition.
Layout also refreshed the session, then asked `choice_rows` to refresh it again.
Browse attaches section metadata through immutable per-item patches, making those
extra passes expensive for large catalogues.

The candidate updates the already-projected first panel's highlight and its
matching view state directly. Layout uses `choice_projected_rows` after its own
refresh. The public `choice_rows` service retains its refresh behavior. That stage did not drop/coalesce input or add a persistent cache or timing
threshold; the final stage later introduced transactional projection ownership.

LuaJIT 2.1.1785763465, x86_64. The focused fixture uses the browse view, an empty
query, 100-column/32-line geometry, and model rows with wrapping descriptions.
Each sample contains 12 next-key transitions followed by layout; numbers below
are CPU milliseconds per transition-plus-layout, not individual latencies.
Each process checks its unchanged oracle six times before timing, and every
sample checks the full resulting semantic frame. Frames matched across variants.

Two invocations per variant ran sequentially in baseline/candidate/candidate/
baseline order. Each invocation contributed ten samples (20 per variant). No
builds or other agent benchmarks ran during measurement. GC remained enabled;
CPU affinity was not pinned.

| Items | Baseline median / best | Candidate median / best |
| ----- | ---------------------: | ----------------------: |
| 100   |          0.658 / 0.506 |           0.428 / 0.364 |
| 1000  |          5.505 / 5.167 |           2.973 / 2.699 |
| 3000  |        20.251 / 19.144 |         10.786 / 10.025 |

Raw samples are in `choice-navigation-2026-09-07.csv`. This measures the actual
choice services, but excludes native transaction/fork overhead, component theme
resolution, terminal presentation and I/O. It reduces a demonstrated cause of
navigation work building up; it does not establish end-to-end release-to-stop
latency in a real terminal. Projection still walks the complete catalogue.

The old `choice-navigation-baseline.patch` is archival and applies only to the
first-stage snapshot; it must not be applied directly to the final source tree.
Use the final report's performance-only baseline patch for current reproduction.
The service harness remains useful as a diagnostic, but these historical timings
must not be attributed to the current implementation.

An initial oracle attempt omitted the keybindings extension and caught an
unrelated fixture instability: the then-current fallback hint chose an alias using `pairs`,
so the cancel hint varied between processes. The retained harness loads the real
keybindings extension. Those failed samples were discarded, not timed as wins.
