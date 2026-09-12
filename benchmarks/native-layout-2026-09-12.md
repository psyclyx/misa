# One native implementation of terminal cell layout

Revision: the worktree at the review follow-up, Debug build, isolated caches. The
change moves `misa.ui.layout` onto the native measurement that the presenter
already uses; it does not change any rendered frame.

## What changed

- `extensions/misa/ui/layout.fnl` no longer carries wcwidth interval tables,
  virama lists, or a grapheme walk. Every primitive now reads cluster boundaries
  and cell widths from `misa.native`, which is `src/width/root.zig`.
- The runtime registers that table as the module `misa.native` in addition to the
  configuration global, so a Lua projection can `require` it instead of
  depending on the global namespace.
- `tests/native-layout.fnl` is the offline stand-in the standalone Fennel harness
  registers under the same name, because `tools/fennel` runs plain LuaJIT with no
  native module. It keeps the previous Lua tables and walk.
- The layout-parity integration case now compares that stand-in with the native
  module over every codepoint, the grapheme corpus, and the cluster arrays, so a
  divergence between the offline measurement and the shipped one fails the suite.

## Evidence

Frames are byte-identical. `benchmarks/native-transcript.py --extension-dir`
selects a saved extension tree while running the same executable, so the only
difference between the two variants is `layout.fnl`. Mixed scenario, 8 samples,
both variants in one session:

| Blocks | Workload | Lua tables (median) | Native (median) | Frame SHA-256 |
| -----: | -------- | ------------------: | --------------: | ------------- |
|      1 | redraw   |               3.831 |           3.678 | identical     |
|      1 | stream   |               4.017 |           4.751 | identical     |
|     16 | redraw   |               4.971 |           5.783 | identical     |
|     16 | stream   |               6.088 |           6.646 | identical     |
|    300 | redraw   |               5.585 |           5.761 | identical     |
|    300 | stream   |               9.282 |           8.626 | identical     |

Every row carries the same SHA-256 of all post-startup frame bytes in both
variants, including warm-up and validation frames, so the swap is
presentation-preserving. Wall-clock frame medians move in both directions within
the ±0.5 ms noise of this machine, so this benchmark cannot resolve a layout
change at this transcript size; it only bounds one.

Behavioral equivalence is checked directly:

```sh
tools/fennel benchmarks/layout-parity.fnl <saved previous layout.fnl>
# layout parity passed: every codepoint width; 500 generated mixed-text cases
```

That compares the previous Lua implementation with the new one over every
codepoint width and 500 generated mixed-text cases through `clip`, `take`,
`width`, the three boundary walks, `wrap-spans`, `fit`, `columns`, and
`wrap-input` with the column count also used as a byte cursor, which is where a
grapheme cut would show. The integration case covers the native side of the same
corpus.

All 77 standalone Fennel cases pass, along with the settlement, threaded
terminal, policy-fault, and Ghostty PTY regressions.

## Cost

`misa.native.clusters` answers with one flat Lua table, so a walk that used to
step through JIT-compiled tables now allocates that array once per call. Measured
with GC stopped, one frame of the 300-block mixed fixture:

| Variant    | Allocated per delta frame |
| ---------- | ------------------------: |
| Lua tables |                   615 KiB |
| Native     |                   674 KiB |

`misa.layout.width` allocates nothing in either variant (64 calls on three
corpora report the same 0.203 KiB of Lua overhead). The extra garbage is the
cluster array for each `clip`, `take`, and `wrap-ranges` call, and it did not
show up as time: the 6000-byte-line and 270 000-byte-message regression
(`tests/long-messages.fnl`) took 1088 ms with the Lua tables and 1089 ms with the
native measurement.

Cheaper answers exist and are not needed yet: the walk could fill a
caller-owned buffer instead of a fresh table, `take` and `clip` could examine a
byte window proportional to their cell budget instead of the whole string, and
the native side could return packed bytes. Each trades simplicity for allocation
that this workload does not spend time on.
