# Property tests

`require("tests.generators")` is a small test helper, not an application dependency.
A generator receives `(random, size)` and returns a sample with `value` and a lazy
`shrinks()` function. Compose generators rather than writing a separate randomized
runner for each subsystem:

```lua
local G = require("tests.generators")
local failure = G.for_all(G.vector(G.integer(-20, 20)), function(items)
  assert(#items <= 8)
end, {seed = 1729, cases = 100, size = 8})
assert(not failure, failure and require("fennel").view(failure))
```

Available generators are `constant(value)`, `elements(values)`, `boolean`,
`integer(low, high)` (inclusive), `one_of(generators)`, `tuple(generators)`,
`vector(generator, minimum=0)`, `map(function, generator)`, and
`recursive(leaf, branch_function)`. Vector length ranges from minimum to minimum
plus size. Recursive children halve the size budget. Integer bounds and their
difference must have magnitude below 2,147,483,647.

Properties pass unless they throw or return false. `for_all` returns nil on
success, otherwise the seed, case number, shrunk value, error, and shrink budget
information. Re-run with the same generator, options, and seed to reproduce.
The default seed is fixed; patch tests also accept `MISA_PROPERTY_SEED`.

Shrinking greedily keeps candidates producing the same error text (or false).
Use stable assertion messages so changing values does not prevent shrinking.
The default limit is 1,000 shrink attempts; the result is not guaranteed globally
minimal. Mapping preserves the source shrink tree; vectors remove elements and
shrink individual elements. `one_of` shrinks within the selected alternative.
Treat generated values as immutable, including values supplied to `constant`.
Keep explicit regression tests for known edge cases alongside properties.
