# Benchmark inputs

Run timing probes sequentially on an otherwise idle machine. Correctness and
startup smoke runs during development are not performance measurements.

Parser and layout comparisons load files that return their public API tables:
`parse`/`new-document` for Markdown, `project` for Markdown layout, and the
ordinary layout operations for text layout. Saved implementations using an older
constructor API need an explicit adapter file returning that API table. The
harness does not infer module construction protocols.

Choice and transcript projection comparisons take declaration catalog tables.
`--extension-dir` selects a complete saved extension tree using the current
plain-data application contract. Profile scripts set this search path before
loading stock composition. Native drivers also expose repository fixture modules
through the generated configuration's Fennel search path.

The startup probe attributes source compilation, module evaluation, installation,
and the first dispatch/projection separately. Declaration assembly is ordinary
module evaluation; there is no separate module-construction phase.

`transcript-profile.fnl` and `projection-phases.fnl` are LuaJIT-side attribution
harnesses: they install the default extensions with a repository fixture, run the
real transaction pipeline, and discard native effects. Their timings exclude
native presentation, terminal output, and scheduling, so they answer "which Lua
phase grew" rather than "how long a frame takes".

Timing harnesses retain their deterministic AST, painted-output, or complete-frame
oracles. Historical reports record the interfaces and measurements at the time
of their experiments.
