# Installed generated-extension startup

Source: `88b068e` plus this build/install integration. Build command:
`zig build -Doptimize=ReleaseSafe -Dcpu=baseline`. Zig 0.16.0 and LuaJIT
2.1.1785763465 on x86_64. Binary SHA256:
`53dbad2e6c22efb97e9a29f74817a3d221aadb349387c53032278f211e1af4ff`.

Reproduce with `python3 benchmarks/installed-startup.py zig-out/bin/misa`.
Ten fresh-process runs per mode used alternating A/B, B/A order on the **same
binary**. Source mode supplies `MISA_EXTENSION_DIR` pointing at the worktree;
installed mode uses generated catalog artifacts. Both use the default profile,
closed stdin, isolated credential/state paths, and empty PATH to prevent provider
CLI execution. Every run exited successfully with exactly
`misa> enter a prompt:\n` on stdout and empty stderr. No builds or other agent
workloads ran concurrently. CPU affinity and OS caches were not controlled.

| Mode                        | Median wall ms | Best wall ms |
| --------------------------- | -------------: | -----------: |
| Source catalog              |        582.319 |      570.858 |
| Installed generated catalog |         82.402 |       78.919 |

This measures process startup through EOF shutdown, not interactive time to the
first usable frame or authenticated-provider readiness. The embedded framework,
state updater, and subscription evaluator still compile from Fennel at startup;
only bundled extensions move to build-time compilation here.

Raw wall seconds:

```text
source      installed
0.579216577 0.082488066
0.602165537 0.081555821
0.593682337 0.081544732
0.582437496 0.079172865
0.577307668 0.078918762
0.589479584 0.082861671
0.570857900 0.083105715
0.577216317 0.083139806
0.582201253 0.083313359
0.583309849 0.082315403
```

Build dependencies include compiler, generator, and source paths. Generated
artifacts are portable Lua source with correlated source lines; original Fennel
files remain installed. Explicit source-directory overrides never probe for
adjacent generated files. Literal Lua and Fennel paths retain their loading path.
The integration harness exercises generated catalog artifacts by default, with
separate source-override and generated-error diagnostic tests. Nix declares host
LuaJIT for generation; a full Nix package build remains a separate verification.
