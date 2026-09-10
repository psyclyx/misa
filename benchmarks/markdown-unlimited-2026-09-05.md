# Markdown without content limits, 2026-09-05

Removed source-byte, block-count, inline-count, inline-depth, and link-length
budgets, the renderer's plaintext overflow fallback, and selection's truncated
source branch. The parser retains full fence language identifiers and list/quote
depths. Deep indentation is fitted to the actual viewport in the renderer.

The old parser was already incremental between blocks. It still repeated two
costly operations within its active block: anchored bracket patterns searched
the same suffix for every unmatched opener, and nested quotes copied the
remaining line after stripping each prefix. Terminator searches now remember
the next match or a failed search as the cursor advances. Quote scanning uses
source offsets. CR-free input avoids replacement passes. Highlighted code line
splitting also uses offsets instead of repeated suffix copies (not included in
parser timings).

## Measurements

LuaJIT from the pinned project environment, default JIT mode, `os.clock` CPU
seconds, 10 samples per variant, alternating forward/reverse order. Benchmarks
ran without builds or agent probes in parallel. No CPU affinity or cache
manipulation. Each variant's unchanged oracle is checked six times before timing;
every timed result is checked against that oracle. The streaming harness compares
complete ASTs against the original full parser at every prefix. Its baseline
now uses the incremental API when available; older baselines without that API
continue using their full parser. The log explicitly identifies the baseline API.

Times below are total CPU seconds for a complete 128-byte-chunk stream (or 128
unchanged redraws), not per-frame latency. Both baseline and candidate columns
use their incremental document API. The raw CSV also contains the candidate full
parser for attribution.

| Workload                | Bytes / updates | Before median / best (s) | After median / best (s) |
| ----------------------- | --------------- | ------------------------ | ----------------------- |
| Paragraph               | 16,500 / 129    | .046241 / .045920        | .036120 / .035611       |
| Headings and paragraphs | 16,800 / 132    | .014163 / .013985        | .003156 / .003051       |
| Fence                   | 17,110 / 134    | .028863 / .028710        | .016744 / .016665       |
| Table                   | 16,830 / 132    | .326475 / .316565        | .317764 / .310311       |
| Unchanged redraw        | 16,500 / 128    | .002793 / .002688        | .002556 / .002510       |

The table remains dominated by reparsing its active block; no table-specific
improvement is claimed. Removing limits is a behavior correction, not a claim
that all streaming work is constant-time. Normalization/prefix comparisons,
block arrays, active block parsing, and final output assembly still scale with
their inputs. Completed block and layout identities remain reusable.

The focused delimiter harness alternates the old and new full parsers. Bracket
and incomplete-link ASTs must match exactly. Quotes intentionally retain their
full depth now, so the harness checks unchanged body inlines and the exact new
depth as well as each variant's deterministic full AST.

| Input              | Bytes  | Before median / best (s) | After median / best (s) |
| ------------------ | ------ | ------------------------ | ----------------------- |
| Unmatched brackets | 2,004  | .021214 / .021083        | .000360 / .000338       |
| Unmatched brackets | 4,004  | .082249 / .081800        | .000696 / .000676       |
| Unmatched brackets | 8,004  | .327736 / .325792        | .001372 / .001337       |
| Nested quotes      | 4,004  | .001468 / .001460        | .000172 / .000168       |
| Nested quotes      | 8,004  | .005142 / .005090        | .000343 / .000334       |
| Nested quotes      | 16,004 | .019037 / .018442        | .000678 / .000663       |

The doubling sweep distinguishes the former quadratic scans from the new
approximately linear scans on these inputs. No general linear-time Markdown
claim follows from this sweep. The saved CSV contains incomplete-link cases and
all raw samples too.

## Correctness and reproduction

The existing 1,009-case AST parity corpus passes. Tests cover byte-split streams,
replacement, old-snapshot immutability, stable block identity, full source past
300,000 bytes, more than 4,096 blocks and 16,384 inlines with styled tails, long
link targets/language identifiers, depth preservation, viewport geometry, and
native highlighting failure without lost code. The complete suite passes with
167 tests passed and three optional grammar skips.

The reverse patch reconstructs the pre-change parser without keeping a second
implementation in the application. Baseline SHA-256:
`5f12c4194344aeaf3b3676e5a91d234e508316c4c494fe5694e6f1872b9557f0`.
Measured candidate SHA-256:
`5681d10c4c6a3a1caa5895dda671168dc60c1e5602b456cf022e0038da36b464`.

```sh
patch -R -o /tmp/markdown-before-unlimited.fnl extensions/markdown.fnl < benchmarks/markdown-unlimited.patch
tools/fennel benchmarks/markdown-parity.fnl /tmp/markdown-before-unlimited.fnl extensions/markdown.fnl
tools/fennel benchmarks/markdown-streaming.fnl /tmp/markdown-before-unlimited.fnl extensions/markdown.fnl 10
tools/fennel benchmarks/markdown-delimiters.fnl /tmp/markdown-before-unlimited.fnl extensions/markdown.fnl 10
```

The former plaintext fallback was removed, not retained as an alternate path.
There are no new thresholds or benchmark-specific parser branches.
