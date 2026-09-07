# Subscription contracts

Subscriptions are pure queries, independent of rendering. Register either a
state read or a computation with explicit query dependencies:

```fennel
{:type :register/sub
 :value {:id :usage/total
         :inputs (fn [_] [[:db/path :status :usage]])
         :compute (fn [inputs query]
                    (local usage (or (. inputs 1) {}))
                    (+ (or usage.input_tokens 0) (or usage.output_tokens 0)))}}

(misa.sub db [:usage/total])
```

A read uses `:read (fn [db query] ...)`, with neither `inputs` nor `compute`.
A computation supplies both `inputs` and `compute`, with no `read`. `inputs`
returns a dense vector of query vectors. The computation receives positional
values plus `inputs.n`, the dependency count: missing values remain nil even at
the end of the vector. Do not use Lua's length operator to count those values.
Constant computations explicitly return an empty dependency vector.

Bundled cost accounting exposes `[:costs/total]` as numeric facts (`usd`,
`responses`, `estimated`, `unknown`), with no formatted text. The dependent
`[:costs/projection]` adds display text; `[:costs/response id]` projects a single
response. The total depends on the response collection, while a response query
depends only on its own record. Unrelated state changes preserve these results
while cached. Existing cost presentation services query this graph rather than
owning a separate cache.

Queries start with a nonempty string ID. Arguments may be strings, finite
numbers, booleans, or nested ordinary data tables. Canonical keys distinguish
types and table contents without relying on delimiters or table addresses.
Functions, metatables, cycles, and holes in query/dependency vectors are rejected.

Dependencies compare by value for scalars and identity for tables. Application
state, query arguments, and subscription results must be treated as immutable.
Callbacks declare dependencies through `inputs`, not recursive scope queries.
Dependency cycles and excessively deep chains report errors.

## Scope ownership

`misa.sub` uses the framework's bounded scope (256 cached queries). Each dispatch
evaluates against a speculative fork. State and memoization commit together;
Lua failure or native rejection discards the fork. Cache eviction may cause
recomputation, so callbacks must not rely on running exactly once.

Other consumers can own a scope explicitly:

```fennel
(local scope (misa.subscription_scope 128))
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

Dispatch passes persistent state directly to handlers and projections; it does
not clone the database or reconcile a mutable draft. Updates enter through
patches. Ordinary tables are not write-protected: callback purity is a contract,
not a sandbox, and in-place mutation cannot be rolled back. Sharing layout work
through subscriptions remains unfinished; this API does not itself establish
selective rendering performance.
