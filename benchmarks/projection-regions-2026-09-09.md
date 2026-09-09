# Independent model and presentation transactions

Baseline: `4d14665`, normal Debug build. Candidate: explicit presentation
transactions, accepted-frame projection owners, transcript/editor dependencies,
and component failure containment. No layout policy moved into Zig.

`tests/projection-regions.fnl` loads the default UI and checks that model dispatch
does not render; typing and scrolling do not rebuild transcript content;
streaming does not rebuild editor input; resize invalidates geometry; rejected
frames do not replace accepted region caches; and a faulty editor component can
be replaced and recovered without breaking root composition. Other tests cover
component exceptions/invalid output, model/effect ordering, native view decoding
failure after model commit, and projection subscription rollback.

The native harness measures keyboard-event to completed synchronized PTY frame,
including native validation and output, excluding terminal-emulator painting.
Each process has six warm-up frames, ten measured frames, and a final untimed
validation frame. Editor redraws are byte-identical after marker normalization;
streaming checks exact agent/transcript content and unchanged block identity.
Every A/B output hash matched. Runs were baseline/candidate/candidate/baseline,
without concurrent tests, builds, or benchmarks. No affinity/cache manipulation.

| Workload | Variant | Run | Median / best / max ms |
| --- | --- | --- | --- |
| Editor update, 300 blocks | Baseline | 1 | 6.676 / 6.387 / 6.978 |
| Editor update, 300 blocks | Candidate | 2 | 5.947 / 5.194 / 8.165 |
| Editor update, 300 blocks | Candidate | 3 | 5.723 / 5.543 / 6.760 |
| Editor update, 300 blocks | Baseline | 4 | 6.578 / 6.165 / 7.829 |
| 32-record transport batch, 300 blocks | Baseline | 1 | 21.469 / 19.173 / 51.036 |
| 32-record transport batch, 300 blocks | Candidate | 2 | 8.887 / 8.586 / 9.844 |
| 32-record transport batch, 300 blocks | Candidate | 3 | 9.729 / 8.470 / 18.653 |
| 32-record transport batch, 300 blocks | Baseline | 4 | 21.710 / 18.936 / 49.662 |

Typing improves modestly end to end: frame composition, validation, and output
remain even when transcript rendering is reused. Streaming improves more by
projecting once after the complete event chain. These are synthetic workloads,
not a claim that all transcripts meet a fixed deadline. Exception containment
does not interrupt infinite loops; both model and presentation use one VM.

Reproduce with `zig build` in each checkout, then use the current harness:

```sh
python3 benchmarks/native-transcript.py /path/to/binary --extension-dir /path/to/checkout/extensions --blocks 300 --mode redraw --rest-ms 25
python3 benchmarks/native-transcript.py /path/to/binary --extension-dir /path/to/checkout/extensions --blocks 300 --mode stream --burst 32 --transport --rest-ms 25
```

The paired CSV preserves each run's output hash and summary statistics. The
earlier eager-projection rejection contract was deliberately replaced, not
bypassed: model/effect failures still reject model transactions, while view
failures reject only presentation transactions.
