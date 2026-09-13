# Canonical operations and streaming evidence

The baseline measurement is rejected as a performance comparison: the old token handler attempted `Op::Append` on a string, but the database append operation accepts lists. Each injected token faulted. It did not produce a correct answer, so its timings or apparent throughput cannot establish a speedup.

Six deterministic repetitions at each size recorded the following work. Each token contains 64 UTF-8 bytes. The baseline measurements were performed before replacing the token handler; the new contract test is `stream_contract_tests::streaming_work_is_linear_and_canonical_state_waits_for_the_log`.

| Path              | Tokens | Accepted append bytes |               Encoded bytes | Token database patches | Token view operations |
| ----------------- | -----: | --------------------: | --------------------------: | ---------------------: | --------------------: |
| Rejected baseline |     64 |                     0 |  82,944 repeated tree bytes |                 failed |                   n/a |
| Rejected baseline |    128 |                     0 | 165,888 repeated tree bytes |                 failed |                   n/a |
| Stream channel    |     64 |                 4,096 |    8,635 stream event bytes |                      0 |                     0 |
| Stream channel    |    128 |                 8,192 |   17,275 stream event bytes |                      0 |                     0 |

Each new repetition asserts every emitted token, exact UTF-8 byte offsets, the final complete body, unchanged database identity and canonical version/work counters throughout streaming, and removal of the ephemeral stream only when the settled message is acknowledged. All six measurements agree at each size. Encoding overhead grows with token count and offset integer width; previous body bytes are never resent in an append.

Canonical mutation work is counted as content builds, structural operations, encoded operation bytes and explicitly requested snapshot nodes. The test/debug differential oracle compares the operation accumulator against a fresh database rebuild after dispatch. Debug production auditing is restricted to documents of at most 128 nodes. Release updates do not invoke the rebuild oracle.

These measurements make no wall-clock speedup claim. Concurrent dependency builds were active. ClientView applies operations to an indexed tree, but `rendered()` materializes a recursive tree for compatibility; frontend retained presentation work is tracked separately and is not established by this server-side result.
