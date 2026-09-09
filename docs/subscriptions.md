# Subscription contracts

Subscriptions are pure queries, independent of rendering. Declare either a
state read or a computation in the `subscriptions` catalog:

```fennel
{:subscriptions
 {:usage/total
  {:inputs [[:usage/session]]
   :compute (fn [inputs query]
              (local usage (or (. inputs 1) {}))
              (+ (or usage.input_tokens 0) (or usage.output_tokens 0)))}}}

(misa.sub db [:usage/total])
```

A read uses `:read (fn [db query] ...)`, with neither `inputs` nor `compute`.
A computation supplies both `inputs` and `compute`, with no `read`. Declare fixed
dependencies as a dense vector of query vectors in `inputs`; these are validated
at installation. When dependencies depend on query arguments, use a function:
`:inputs (fn [query] [[:db/path :costs :responses (. query 2)]])`.
Its returned vector is validated during evaluation. The computation receives positional
values plus `inputs.n`, the dependency count: missing values remain nil even at
the end of the vector. Do not use Lua's length operator to count those values.
Constant computations declare `:inputs []`.

`compute(inputs, query, previous?)` may receive its previous immutable output as
an incremental-computation hint. It must produce the same semantic result when
the hint is absent: eviction, clearing, and a new consumer can all remove it.
Never mutate or retain a history chain through the hint. Forks share prior
outputs, but newly computed output belongs only to the evaluating scope.

Bundled cost accounting exposes `[:costs/total]` as numeric facts (`usd`,
`responses`, `estimated`, `unknown`), with no formatted text. The dependent
`[:costs/indicator]` supplies a typed money fact without formatting;
`[:costs/response id]` projects a single
response as a typed money fact (`amount`, `currency`, `pending`, `estimated`,
`unknown`), not display text. `[:costs/responses]` incrementally projects the response collection,
preserving each result whose record is unchanged. Point queries depend on that
collection; bundled transcript lookups share the collection directly, avoiding
one scope entry per historical response. Unrelated state changes preserve results
while cached. Existing cost presentation services query this graph rather than
owning a separate cache.

Entries in the `indicators` catalog declare named queries through `query`. The
`[:indicators/model]` subscription composes those dependencies into ordered
records containing immutable `fact` data and configured presentation metadata.
Only nil omits a fact; typed false and zero remain visible. Width, theme, hover,
and `[:animations/presentation role]` do not participate in domain fact queries.
The removed `costs/projection` and indicator value-callback APIs have no shim.

Queries start with a nonempty string ID. Arguments may be strings, finite
numbers, booleans, or nested ordinary data tables. Canonical keys distinguish
types and table contents without relying on delimiters or table addresses.
Functions, metatables, cycles, and holes in query/dependency vectors are rejected.

Dependencies compare by value for scalars and identity for tables. Application
state, dependency declarations, query arguments, and subscription results must be treated as immutable.
Callbacks declare dependencies through `inputs`, not recursive scope queries.
Dependency cycles and excessively deep chains report errors.

## Scope ownership

`misa.sub` uses the framework's bounded scope (256 cached queries). Each dispatch
evaluates against a speculative fork. State and memoization commit together;
Lua failure or native rejection discards the fork. Cache eviction may cause
recomputation, so callbacks must not rely on running exactly once.

Other consumers can own a scope explicitly:

```fennel
(local scope (misa.subscriptions.scope 128))
(scope.query db [:usage/total])
(local speculative (scope.fork))
;; Use the fork for speculative work; retain it on success or close it on failure.
(speculative.close)
(scope.close)
```

`clear()` drops cached values, `size()` reports the cache entry count, and
`close()` releases entries and prevents further queries. Forking copies the
bounded cache index while sharing immutable entries; its cost is proportional
to the cache size, not application-state size. Explicit consumer scopes are
consumer-owned: the framework does not automatically roll them back.
Capacity defaults to 256 only when omitted/nil and otherwise must be a positive
finite integer. False, fractions, infinities and NaN are rejected. New scopes
start empty; inherited memoization is private to `fork()`. A callback cannot
query, clear, close or fork its evaluating scope: dependencies belong in `inputs`.

Dispatch passes persistent state directly to handlers and projections; it does
not clone the database or reconcile a mutable draft. Updates enter through
patches. Ordinary tables are not write-protected: callback purity is a contract,
not a sandbox, and in-place mutation cannot be rolled back. Sharing layout work
through subscriptions uses collection ownership rather than one flat scope
entry per historical item. `misa.components.project` owns semantic component
output, incremental hints, and resolved views in one collection subscription;
syntax and cost enrichment likewise use shared collection projections.
Model preparation and viewport assembly still perform general traversal; this
API does not imply O(1) streaming or guarantee a frame budget. The bounded
[startup and mixed-transcript check](../benchmarks/architecture-handoff-2026-09-07.md)
records measured scope and limitations separately from subscription correctness.
