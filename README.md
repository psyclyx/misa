# misa

misa is a small event-driven coding-agent harness built with Zig 0.16,
system LuaJIT, bundled Fennel 1.6.0, and system tree-sitter. Terminal presentation
uses Zig; bounded image decoding uses system libpng and libjpeg-turbo.

Configuration is ordinary Fennel data. Copy the stock application, change its
maps, and return it. For the defaults with one component replaced, save this as
`config.fnl`:

```fennel
(local app (misa.snapshot (require :misa.standard)))

(fn header [_ _]
  {:lines [{:spans [{:text "my misa"}]}]})

(tset app.definitions.components :default.root.header {:render header})
app
```

Run `misa --config ./config.fnl`. A new map key adds an implementation; assigning
an existing key replaces it; assigning `nil` removes it. Arrays are ordinary
arrays, so `[]` clears one. `misa.snapshot` copies nested tables while retaining
functions, leaving imported stock data unchanged.

Settings are equally direct:
`(tset app.config.models :default "provider/model")`. Implementation choices live
in `app.definitions`; runtime options live in `app.config`. The host validates
and installs the finished application once. A minimal application is
`{:config {} :definitions {}}`.

```sh
zig build
zig build test
zig build test-integration
zig build test-nix
zig build -Doptimize=ReleaseSafe
zig build run -- --config config/default.fnl hello
```

Use `nix-shell -A shell` for the pinned development dependencies. `zig build test`
runs native unit tests and the named Zig integration cases in `tests/integration/`.
The suite owns temporary configurations, child processes, deadlines, and output
assertions; extension fixtures live in `tests/integration/fixtures/*.fnl`.

Formatting is defined by `treefmt.toml` and pinned by the same shell, so it is
handled by `treefmt` rather than by invoking each formatter directly. Enable the
tracked pre-commit check once per clone:

```sh
git config core.hooksPath githooks
```

It formats only the staged files, so unrelated unformatted work never blocks a
commit. Because `treefmt` formats the working tree rather than the staged blob,
the hook also refuses to commit when the two disagree: commit the whole file
rather than a partial stage. `git commit --no-verify` bypasses it.
Application integration runs `misa-fixture`, composed with fixture HTTP and
authentication dependencies. HTTP requests fail as `UnmatchedHttpFixture`
before credentials are loaded; OAuth login has no fixture implementation.
Authentication storage tests use temporary stores, and CLI authentication tests
register their fixture executable explicitly. Fixture environments inherit only
tool paths, locale, terminal type, and the grammar dependency; HOME, XDG paths,
and provider configuration directories belong to the fixture.
Provider command effects use a separate `provider/process` capability. The
fixture implementation only runs absolute executables registered by the fixture
owner under its temporary root, with the owner's explicit environment; it never
falls back to an installed provider CLI.

Provider policy tests inspect emitted effects and supply synthetic responses.
The native process and MCP adapter tests intentionally execute local shell
fixtures: this is dependency separation, not an OS security sandbox or a promise
about arbitrary new test code. Directly invoking the production executable or
writing a test that spawns a live provider bypasses the fixture composition.
Run standalone policy tests with `tools/fennel tests/runtime-state.fnl`. The same
runner supports the Fennel benchmark scripts and uses the bundled compiler.

Build the fixture executable with `zig build fixture-app`. Optional
terminal-protocol regressions run with `python3 tests/ghostty-input.py
zig-out/bin/misa-fixture`, `python3 tests/settled-frames.py zig-out/bin/misa-fixture`,
`python3 tests/threaded-terminal.py zig-out/bin/misa-fixture`, and
`python3 tests/terminal-failure.py zig-out/bin/misa-fixture`. They
use a PTY and local provider fixtures, with no account or network dependency.
`python3 tests/policy-fault.py zig-out/bin/misa-fixture` checks that an
interactive session reports and survives an invalid effect and a raising
handler.
`python3 tests/http-cancellation.py zig-out/bin/misa` exercises cancellation,
idle timeouts, and compressed responses through the real HTTP transport using
a local server; build the production executable with `zig build` first.
Pass `--installed` to `tests/ghostty-input.py` to verify the executable's installed
catalog rather than loading extensions from the worktree. This mode uses the
catalog installed alongside `misa-fixture` by `zig build fixture-app`.

Without an explicit configuration path, misa loads the installed `share/misa/default.fnl`, which
selects all shipped real providers, Claude Code by default, coding tools, model
policy, authentication commands, the agent, and the UI as a useful coding
profile. Run `misa login claude` first.
`--config PATH` takes precedence over `MISA_CONFIG`, which takes precedence over
that installed default. Arguments not consumed by `--config` are exposed as
`cofx.argv`.

Bare standard IDs use `MISA_EXTENSION_DIR` when set, otherwise the absolute
`share/misa/extensions` path compiled from `zig build --prefix`. A user
extension directory (the `misa/extensions` subdirectory of
`XDG_CONFIG_HOME`, otherwise `$HOME/.config`) is searched before the installed
catalog and after the configuration's own directory, so a configuration can
`require` its own modules by name without an environment override. Relocated or
copied installations must set both `MISA_CONFIG` and `MISA_EXTENSION_DIR`; misa
never discovers its own executable path. Values containing `/` or ending in
`.fnl` or `.lua` are literal custom paths. Standard extensions and the embedded
event framework are written in Fennel. The compiler is embedded in the executable;
loading installed or custom Fennel extensions requires no external compiler.
Lua extensions remain supported by the same VM boundary.
For Fennel/Lua configuration, ordinary `require` searches its directory and the
standard module directory. Return `{config=..., definitions=...}`; only `config`
crosses the JSON boundary, while definition callbacks remain in the Lua VM.
The build requires a host `luajit` executable to translate bundled extensions and
the embedded runtime core to portable Lua source. Installed catalog IDs use generated `.lua` files; explicit
`MISA_EXTENSION_DIR` overrides continue to use `.fnl` sources so development edits
cannot be hidden by stale generated files. The original sources are installed
alongside generated files, whose line layout is correlated to the Fennel source
for diagnostics. The embedded runtime core loads translated Lua at startup;
the embedded compiler remains available for custom Fennel modules. `zig build run`
uses those freshly built Lua modules too; set `MISA_EXTENSION_DIR` to the source
`extensions` directory explicitly when debugging runtime Fennel compilation.

## Event, coeffect, effect, and view contract

An application is ordinary data: `{config, definitions}`. Definitions are maps
of catalogs, each containing named entries. The host evaluates the configuration,
copies only its JSON settings into native code, and installs the callback-bearing
catalogs in Lua once. Installation validates and seals the complete application.
There is no registration program to execute during startup or event handling.

A minimal application can return its definitions directly:

```fennel
{:config {:example {:enabled true}}
 :definitions
 {:events {:example/start
           {:event :app/start :priority 0
            :handler (fn [db event cofx]
                       {:patch {:example {:enabled cofx.config.example.enabled}}
                        :fx [{:type :dispatch :event {:type :example/ready}}]})}}
  :services {:example.enabled (fn [db] db.example.enabled)}}}
```

Implementation modules export named functions and domain values. Stock composition
under `misa.standard` associates those values with catalog IDs, event types,
keyboard bindings, and ordering. Importing `misa.editor`, for example, gives
editor operations; importing `misa.standard.editor` gives the stock
editor catalog fragment. Neither import installs anything.

Use ordinary map operations to customize a copied application:

```fennel
(local app (misa.snapshot (require :misa.standard)))
(local replacement (. (require :my.editor) :input))

(tset app.definitions.components :default.editor.input replacement)
(tset app.definitions.keybindings :global/toggle_verbose :default ["alt+v"])
(tset app.definitions.commands :/clear nil)
app
```

The replacement component is a data record such as `{:render render-input}`.
Changing the value at the existing component key keeps its role association;
adding a new component ID makes another implementation available. Runtime
component selection and persistence still use `config.components.roles` and
`components/swap`.

Stock fragments are ordinary catalog maps. Import only the fragments needed by
a smaller application and combine their entries explicitly:

```fennel
(local definitions {})
(each [_ fragment (ipairs [(require :misa.standard.json)
                           (require :misa.standard.agent.stream)
                           (require :misa.standard.protocols.openai)
                           (require :misa.standard.providers.openai)])]
  (each [kind entries (pairs fragment)]
    (when (not (. definitions kind)) (tset definitions kind {}))
    (each [id value (pairs entries)]
      (tset definitions kind id value))))

{:config {} :definitions definitions}
```

This example selects transport services and an OpenAI adapter; a conversation
application also needs agent, model, and interaction policies. Each fragment
shows its complete wiring. [`misa.standard`](extensions/misa/standard/init.fnl)
shows the stock combination, and
[`misa.standard.settings`](extensions/misa/standard/settings.fnl) shows the
runtime defaults. Keep function definitions in implementation modules and choose
registration identities in application composition.

The stock application is assembled one directory at a time. A directory's
`init.fnl` is the module for that directory: its own declarations from the sibling
`core.fnl`, if it has any, merged with its children. It names only modules under
its own directory, so `misa.standard.editor` is the stock editor,
`misa.standard.editor.core` is the editor's own declarations, and
`misa.standard.editor.history` is one child. Growing a directory means editing that
directory's module, never a list that spans directories.

Source names follow ownership paths: `misa.editor` resolves to
`misa/editor/init.fnl`, and `misa.editor.history` to `misa/editor/history.fnl`.
Source names, service paths, event types, and registration IDs are separate
contracts. Moving editor code does not rename `misa.editor.layout` or
`editor/restore`. See the [standard library layout](extensions/README.md).

The installed catalog and service reference is generated from the stock
application: [docs/catalogs.md](docs/catalogs.md) lists every catalog, its
entries, and each entry's shape, and [docs/services.md](docs/services.md) lists
every installed service path. Both come from
[`tools/generate-docs.fnl`](tools/generate-docs.fnl), which the suite runs in
`--check` mode so a stale document fails the build. The layering, the Zig/Lua
boundary, and the migration plan for the session kernel are stated in
[docs/architecture.md](docs/architecture.md).

Catalogs use the following entries (the outer map key is the entry's ID):

| Catalog                                                    | Entry                                                             |
| ---------------------------------------------------------- | ----------------------------------------------------------------- |
| `events`                                                   | `{event, handler, priority?}`; handlers receive `(db,event,cofx)` |
| `routes`                                                   | `{event,priority,context,resolve}`; one pure event route          |
| `coeffects`                                                | `function(cofx,event,db)`; derives a transaction input            |
| `effects`                                                  | `function(effect,cofx,db)`; translates policy effects             |
| `views`                                                    | `function(db,context)`; exactly one root semantic view            |
| `view-layers`                                              | `{handler=function(db,context)}`; overlay, exclusive, or dock     |
| `subscriptions`                                            | `{inputs,compute}` or `{read}`; a pure query                      |
| `projections`                                              | `{inputs,render}`; an independently cached presentation owner     |
| `services`                                                 | A function or immutable value, named `namespace.member`           |
| `serializers`                                              | `{accepts,serialize}`; a model request-option serializer          |
| `models`, `auth-providers`, `commands`, `actions`, `tools` | Domain declarations; their owning APIs validate and expose them   |
| `completions`                                              | `{group,value={value,label?,description?}}`                       |
| `requirements`                                             | An array of service paths, keyed by consumer ID                   |
| `validators`                                               | A pure `(id,value)` validator, keyed by catalog name              |

Additional catalogs belong to their domain modules: components, themes, tool
presentations, input policies, and protocol adapters remain ordinary named data.
`misa.catalog(kind)` returns their installed map; `misa.catalog-entries(kind)`
returns entries in priority and ID order. Both return borrowed immutable values.
Service results and query inputs follow the same ownership contract unless an API
explicitly documents fresh mutable storage. Copy or use `misa.patch` to derive a
changed value; never edit a borrowed catalog, model, projection, or service table.
A catalog kind the host does not recognize is installed and readable through
`misa.catalog(kind)` and `misa.catalog-entries(kind)`; it simply has no
registration semantics of its own.

Event handlers run by ascending finite integer `priority` (default zero), then
by their stable definition IDs. Coeffects run by ID; do not encode hidden module
load-order dependencies. Requirements such as
`{:requirements {:feature ["themes.current" "components.render"]}}` are checked
after services have been installed, with a diagnostic naming the missing consumer
and path. Runtime effects cannot add definitions or reopen installation.

Handlers return nil or `{patch=<map>, fx=<ordered array>}`. Patches recursively
merge maps and replace nonempty arrays and scalars. Empty patches do nothing;
`(misa.replace {})` clears a collection and `misa.delete` removes a key.
`(misa.at index value)` writes one array element, `misa.append value` grows an
array at its end, and `misa.append-all values` appends every element of a
nonempty array in order. All three are bounded by the current length, reject a
control nested in replacement data, and keep the target array when the write
changes nothing. Appending is the one patch operation that is not idempotent.
`misa.json-null` stores JSON null. Returning a whole `db` is rejected. State
contains finite JSON data, never callbacks, metatables, cycles, or patch controls;
unchanged branches retain identity. Allocate new values rather than mutating
handler inputs. Effects run only after the model transaction commits. Emit an
explicit continuation event when startup work needs other owners' settled state.

Base coeffects include immutable `config` and `argv`, terminal facts, and
`clock={wall_ms,monotonic_ms}`. Native code supplies both clocks; use monotonic time
for elapsed durations. The runtime coeffects also include `host={executable,config_path}`
for subprocess bridges. Stock service and effect adapters can read installed settings
with `misa.configuration()` and pass explicit data into policy functions. This
returns a borrowed immutable value. Startup IO is described by effects from
`app/start` handlers. Policy stays in Lua; native code owns IO, cancellation,
JSON transport validation, and terminal frame decoding.

Routes read their declared subscription context and return a semantic event or
nil. The highest-priority claim wins; equal-priority claims are an error. The
winning event reaches ordinary handlers in the same transaction without recursive
routing. Routes return no patches or effects. Subscriptions use
`misa.sub(db, [id, ...args])`; see [subscription contracts](docs/subscriptions.md)
for nullable inputs, consumer scopes, and speculative commit/rollback.

Protocol modules export request, discovery, serialization, and stream operations.
These consume explicit transport settings and normalized request/event data; stock
provider fragments associate them with effects and completion events. The asynchronous
`syntax/highlight` effect accepts `id`, `language`, `source`, and `completion`,
with an optional `timeout_ms` (1–60000; default 1000). Its completion event has
`id`, `ok`, and `data` containing ordered
`{start_byte=<zero-based>, end_byte=<exclusive>, capture=<semantic name>}`
ranges. Captures use a finite generic vocabulary (`comment`, `string`, `number`,
`keyword`, `type`, `function`, `constant`, `variable`, `property`, `tag`,
`attribute`, `operator`, `punctuation`, `escape`, and `embedded`). Highlighting
is derived data: the `misa.transcript.syntax` extension tracks pending requests and accepted
revisions, immutable parsed documents, and accepted capture arrays in transactional
state. It consumes explicit `transcript/updated` notifications, not a global
before/after interceptor; projections never schedule highlighting.
The `syntax/projections` subscription incrementally projects document
entries, retaining unchanged results. Point queries use `syntax/projection`;
collection consumers call `misa.syntax.all(db)` once and pass that
immutable snapshot to `misa.syntax.for-model(snapshot, model)`. The latter is a
pure lookup with source matching, not a subscription evaluation. Transcript
enrichment therefore avoids both per-document scope eviction and repeated
evaluation of the same collection query. Retained states do not depend on an external
cache. Projection results are immutable, including when adding layout options.
Source parsing and grammar
loading run on native workers with a reusable parser cache. Missing, unknown,
incompatible, or timed-out grammars produce plain-text fallback. Source is
limited to 1 MiB. There is no synchronous highlighting API on `misa`.
Recursive dispatch is
unavailable. State commits retain unchanged branches by identity, so subscriptions can cheaply
reuse projections that do not depend on the changed paths. Handlers, coeffect derivations,
routes, and projections receive persistent state directly: dispatch does not clone or
reconcile the database. State changes belong in event handlers as patches.
These are ordinary tables, not write-protected proxies; mutating shared input violates the
contract and cannot be rolled back. Valid patches and subscription memoization commit only
after native validation succeeds. Views receive the pending state before that validation.
Native frame construction and terminal writes follow asynchronously; output failures end
the session. Extensions must
not write or render: `io`, `os`, and `print` are unavailable. Ordinary Lua
source composition (`require`, `load`, `loadfile`, and `dofile`) remains
available, while native `package.loadlib`, FFI, and JIT access are disabled.

`config.runtime.max_dispatch_chain` bounds each synchronous event chain (default
1024, range 1–1000000). On overflow, interactive sessions discard its remaining
queued events, preserve previously committed state/effects, publish the last
valid view, and resume input. A `runtime/dispatch-limit` event carries `limit`,
`event_type`, and `text`; the standard messages extension displays it. Headless sessions exit
with `DispatchChainLimitExceeded`. This protects against self-dispatch loops,
not a callback that never returns or a blocking native/file-loader call. LuaJIT
remains enabled; arbitrary plugin isolation requires a separate worker/process.
Native operations require concurrent worker execution. If a worker cannot be
started, the operation completes with an error instead of running inline on the
session thread.

Component theme resolution constructs new output records and shares unchanged
semantic data. Cached message spans stay semantic across theme changes. The
terminal adopts prepared frame bytes and click maps after successful output;
the session transfers the semantic view arena to the terminal owner. State,
subscription results, and component inputs are shared immutable values by
contract, not defensive copies. Callback mutation is a contract violation and
cannot be rolled back; rejected transactions discard pending patches and queries.

HTTP deadlines describe only signals the transport can observe: `first_byte_ms`
includes DNS and connection establishment, `idle_ms` applies after response-body
activity begins, and `overall_ms` bounds the complete operation. There is no
separately configurable connect deadline.

A transport failure that leaves the request unjudged (a refused connection, a
reset read) and the transient statuses 408, 425, 429, 500, 502, 503, and 504 are
attempted again. `retries={attempts=2,backoff_ms=500,max_ms=8000}` bounds that:
`attempts` counts the extra attempts, the wait doubles per attempt, and a
server's `Retry-After` delay takes precedence up to `max_ms`. Cancellation and
timeouts are never retried, and a stream that already delivered a record is
never replayed, so policy never sees a duplicated record.

The fixed native effects are:

- `{type="dispatch", event=<table>}`
- `{type="syntax/highlight", language=..., source=..., completion=..., id=..., timeout_ms=?}`
- `{type="process/run", argv={<strings>}, completion=<event type>, id=<string>, timeouts={startup_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="provider/process", argv={<strings>}, completion=<event type>, id=<string>, timeouts={startup_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="http/request", url=..., json=..., credential=..., completion=..., id=..., timeouts={first_byte_ms=?,idle_ms=?,overall_ms=?}, retries={attempts=?,backoff_ms=?,max_ms=?}}`
- `{type="file/read", path=..., completion=..., id=..., start_line=?, max_lines=?, anchored=?}`
- `{type="file/list", path=..., completion=..., id=...}`
- `{type="file/write", path=..., content=..., completion=..., id=...}`
- `{type="file/edit", path=..., content=..., replacement=..., completion=..., id=...}`
- `{type="file/edit_lines", path=..., snapshot=..., start=..., end=..., position=..., content=..., completion=..., id=...}`
- `{type="json/decode", source=..., completion=..., id=...}`
- `{type="terminal/read"}`
- `{type="clipboard/write", text=<up to 1 MiB>}`
- `{type="image/load", path=..., id=..., completion=...}`
- `{type="image/paste", argv=<optional clipboard command>, id=..., completion=...}`
- `{type="input/protected", id=..., correlation=..., completion=...}`
- `{type="timer/start", interval_ms=<10..60000>, completion=..., id=...}` / `{type="timer/stop", id=...}`
- `{type="operation/cancel", id=...}` / `{type="operation/finish", id=...}`
- `{type="auth/command", action=..., provider=..., strategy=..., profile=...,
account=?, completion=..., interaction=..., id=...}` / `{type="auth/respond",
id=..., correlation=..., action=..., value=...}`
- `{type="state/load", namespace=..., completion=...}` / `{type="state/save", namespace=..., data=...}`
- `{type="conversation/append", conversation=<id>, entries=<array of {kind=..., data=...}>, metadata=?, completion=..., id=...}`
- `{type="conversation/load", conversation=<id>, after_seq=?, limit=?, completion=..., id=...}`
- `{type="conversation/list", limit=?, completion=..., id=...}`
- `{type="conversation/request", id=<request id>, conversation=?, kind=<turn|side|probe>, provider=...,
model=..., status=..., provider_id=?, parent_request_id=?, cost_kind=?, cost_micros=?, cost_currency=?,
input_tokens=?, output_tokens=?, cache_read_tokens=?, cache_write_tokens=?, ttft_ms=?,
finished_at_ms=?, settings=?, usage=?, cost=?, metadata=?, completion=...}`
- `{type="view/commit", lines=<semantic lines>}`
- `{type="app/quit"}`

A policy fault is contained in an interactive session and fatal in a headless
run. An unknown native effect, an invalid handler result, or a handler that
raises rolls its transaction back and reports `runtime/effect-error` or
`runtime/handler-error`, carrying `event_type` and `text`; the session keeps
reading input afterwards. A headless run exits with the same diagnostic as
before. A rejected frame reports `runtime/presentation-error` once per episode,
keeps the last valid frame, and retries after a later event. `http/request` injects credentials by
ID inside Zig, so secret bytes never cross into Fennel policy. Provider adapters
use `provider/process`, whose execution dependency is selected by application
composition. General-purpose tools use `process/run`. Both invoke direct argv,
never a shell, captures stdout and stderr with 1 MiB bounds, and never inherits
the terminal output. The shell tool explicitly translates its command to
`["sh" "-lc" command]` in Fennel; filesystem and process isolation are concerns
of the environment launching Misa, not of the tool extension. File effects use
paths exactly as supplied and bound file content to 1 MiB. Captured tabs are
normalized to spaces and malformed UTF-8
is repaired before completion events are dispatched. Completion events include
`ok`, `status`, `stdout`, `stderr`, and `id`. With
`stdout_format="json_lines"`, successful output is decoded into an ordered
`records` array instead of being returned as an opaque string. `process/run`
also accepts bounded `stdin` text or one `stdin_json` value; the latter is
serialized with a trailing newline for JSONL subprocess protocols.

`conversation/append` persists durable, ordered conversation records to a SQLite
database opened in WAL mode. Each append is one transaction, so concurrent Misa
processes serialize without lost updates or duplicated sequence numbers;
`busy_timeout` plus bounded retries absorb ordinary lock contention. Entry `data`
is any JSON value and is stored as text, so the transcript model can evolve
without a schema migration; the completion event reports `{count, last_seq}`.
The database path is `$MISA_CONVERSATION_DB`, otherwise
`$XDG_STATE_HOME/misa/conversations.sqlite3`, otherwise
`$HOME/.local/state/misa/conversations.sqlite3`.

`conversation/load` reopens one conversation as a bounded page of entries in
ascending order, with its header metadata, fork provenance, and the attempts
that branch issued; `conversation/list` returns recent headers with their entry
counts. Both run on a worker like the append, because opening the database can
wait on another process's write lock.

Provider calls record themselves. The effect that asks for one declares the
attempt it is recorded as — `:attempt {:conversation=<id>, kind=<turn|side>}`, the
rest filled in from the effect's own id, provider, and model — and the transport
writes the row before the call starts and settles it when the call ends. A
provider process that declares no attempt is refused, and a `provider.*` effect
whose translation is a call must declare one; an adapter that answers by
dispatching is not a call and is not asked. The completion event carries
`attempt_id`, and the row settles `unknown` for cost unless policy reports a
figure.

`conversation/request` records one provider attempt, which is what makes cost
and usage facts rather than session memory. A row is written when the attempt
starts and enriched when it finishes; an omitted field keeps the value already
recorded, so a write that only adds usage cannot relabel or cheapen an attempt,
and `kind` is fixed when the row is created.
The attempt is named by its branch — `<conversation>/<request id>`, or the
caller's id alone when there is no conversation — because a request id is only
unique within the branch that used it. A name a finished attempt already used
is recorded beside it, as `<id>#2`: a resumed session numbers its requests from
the start again, and merging the two rows would lose the newer attempt's cost.
A name an unfinished attempt still holds is refused as `AttemptInProgress`,
because two live attempts under one name is a mistake rather than a resume.
`kind` says what the call was for — `turn` is a step of the conversation,
`side` is a call policy made outside the turn, `probe` is an adapter asking a
provider about itself — and `status` says where it stands: `started` while it
is in flight, then `ok`, `error`, or `cancelled`. An attempt that never
finished is therefore visible as a fact, which is what lets a loaded branch
report a request that was made and never came back instead of mistaking it for
a turn about to start.

The stock profile journals canonical history through `misa.conversation`. A
session names its conversation from `config.conversation.id` or from its own
start clock, and the log records each canonical message as soon as it is final —
the prompt when the turn starts, the assistant message when its response ends,
each tool result when its tool reports — labelling the conversation from its
first user message. One message per append is also why the native entry limit on
an append is never in play: a turn is recorded in order, message by message,
instead of in a batch whose size grows with the turn. A shorter history
means the branch was replaced, as compaction does, so the log records an
explicit reset entry instead of silently diverging from canonical history.
`/resume` lists the stored conversations through the ordinary picker and
installs the chosen one as canonical history, replaying its user, assistant,
and tool messages into the transcript; `/resume ID` loads one directly.
`config.conversation.enabled` (default `true`) turns journalling off, `id`
pins one conversation across sessions, `label` names it explicitly, and
`list_limit` bounds the picker. The log is a journal of turns: resuming
restores the branch a session wrote, not the terminal scrollback.

The preference document behind themes, component roles, model selection, editor
history, and choice recency is a separate atomic JSON file: `$MISA_STATE_FILE`,
otherwise `$XDG_STATE_HOME/misa/state`, otherwise `$HOME/.local/state/misa/state`.

A view is modest semantic data. The root UI composes ordered region descriptors
with shared height budgets and cursor placement. Dialog fields, transcript roles,
and tool status styles are data tables interpreted by their component owners:

```fennel
{:lines [{:spans [{:text "working"
                   :style {:foreground :default :dim true}}]}]
 :cursor {:row 1 :byte 0 :shape :bar}} ; or nil cursor
```

Cursor shapes are `bar` (the default, used for insert mode) or `block` (normal
and visual modes). Both are steady native cursors, so redraws and activity
animations cannot restart a blink phase. Terminal exit restores the default shape.

At the native boundary, `style` is a validated record with optional
`foreground`, `background`, `bold`, `italic`, `dim`, `strikethrough`, and `underline` fields.
A color is `default`, one of the 16 ANSI names (`red`, `bright_blue`, and
so on), or `{r=0..255,g=0..255,b=0..255}`. Missing style fields inherit the
reset terminal defaults. The shipped theme uses terminal-default body text,
RGB accents and muted RGB message backgrounds. Spans
may also carry `link=<URL>`. Interactive presentation wraps visible linked text
in OSC 8 open/close sequences; links are limited to 4096 bytes and rejected if
they contain terminal controls. Noninteractive output emits neither SGR nor
OSC sequences. Spans may independently carry `action=<registered action ID>`;
the native cell hit map dispatches clicks through the ordinary action registry.
Hit targets update only when their frame is successfully presented.

Text must be valid UTF-8 and may not contain controls, ESC, CR, or LF. Cursor
rows are one-based; `byte` is a zero-based UTF-8 boundary in the concatenated
spans of that row. Zig alone converts it to a terminal cell column and clamps it
to the presentable width. Zig owns clipping and ANSI mapping; noninteractive
output strips styles.

## Terminal architecture and limitations

`src/session/root.zig` owns a non-reentrant FIFO event loop and parses effects
once into a closed native union; `src/capability/process.zig` owns direct
process execution and captured-output normalization, while
`src/capability/file.zig` owns bounded file operations. Fennel owns canonical application
state. Fennel handlers and semantic view projection share one serialized VM,
with separate transactions. Dispatch validates effects before committing the
model and executing effects. After the synchronous event chain settles, a
presentation transaction projects the committed model, validates the frame,
and transfers an owned view to the terminal. Rendering failure preserves the
committed model, executed effects, last valid frame, and accepted projection
caches. It retries after a later event rather than spinning on a broken view.
Presentation receives `argv`, `config`, terminal facts, and a fresh clock;
event-derived custom coeffects belong to model handlers. Views that need those
facts should consume committed model state through declared projection inputs.

Extensions can register a projection owner:

```fennel
{:projections {:feature.project
 {:inputs (fn [db] {:feature db.feature :theme db.themes})
 :render (fn [db context] (render-feature db.feature context))}}}
```

This installs an ordinary service at `misa.feature.project`. The owner must
declare every state dependency used by its render callback; context fields are
also compared. Inputs and outputs follow the framework's immutable-value
contract. Calls can compose subscriptions and other projections. Accepted frames
retain one result per registered owner; rejected frames discard speculative
results. Model handlers may reuse an accepted result, but cannot publish new
presentation caches. The transcript owns its content/layout dependencies, while
the editor owns its input projection. Typing and scrolling reuse transcript
layout; streaming reuses unchanged editor input. Viewport slicing still responds
to editor height and selection changes.

Component rendering is a local failure boundary. Exceptions and malformed
semantic output produce a bounded placeholder with `component_error` diagnostic
metadata, including for nested children. Other components remain available, and
changed models, contexts, or implementations permit recovery. Native frame
validation remains the backstop for invalid final geometry and encodings.
An infinite loop or a long native callback is not isolated by exception handling:
the shared VM still serializes execution. This separation does not promise a
hard input-latency bound or introduce a second rendering VM.

`src/terminal/driver.zig` runs a separate terminal thread. Its latest-view mailbox
coalesces frames before native rendering; image updates use the actually displayed
image cache. Frames prepared for obsolete dimensions are discarded. The driver
owns terminal reads, decoder deadlines, frame construction, hit testing, and all
terminal writes. Decoded input uses a 64-entry queue plus one bounded read batch;
the session interprets one event per settled chain so protected-input transitions
remain ordered. Mouse actions retain the hit map displayed when their batch was
decoded. Clipboard, committed output, input discard, and generation-checked
handoff use acknowledged commands. Shutdown joins the driver before restoring
terminal modes; blocked stdout can delay that join until its write completes.
Panics restore terminal modes and leave the alternate screen before printing
diagnostics; fatal-signal cleanup also handles SIGABRT.

Potentially blocking HTTP, process, authentication, file, credential, and
persistent-state work runs in cancellable native workers and reports completion
errors as events. Worker publication uses bounded per-operation queues. The
session waits on terminal and operation wakeup pipes and timer/operation deadlines.
The terminal thread waits on stdin, commands, frame and decoder deadlines, with
a 100 ms maximum wait to observe resize signals delivered to another thread.
`src/wakeup/root.zig` supplies the shared nonblocking notification mechanism.
`src/terminal/root.zig` owns tty lifetime; `terminal/input.zig`,
`terminal/presenter.zig`, and `width/root.zig` own their respective mechanisms.

The interactive presenter owns an alternate screen and repaints bounded
semantic frames using absolute cursor positioning. Leaving Misa or temporarily
handing the terminal to an external CLI restores the previous screen. The
transcript remains in application state and supports scrolling and structural
selection. Live frames use the full terminal width with autowrap temporarily
disabled; plain committed output may wrap naturally. The presenter encloses
text, image placement, row clearing, and cursor changes in synchronized-output
sequences. It presents only settled synchronous dispatch chains, at a 16 ms
coalescing cadence, and skips identical frames. Components need no refresh or
synchronization logic. Managed input uses a steady native bar cursor in insert
mode and a steady block in normal/visual mode, avoiding blink resets during
redraw. Terminal handoff and exit restore the default cursor shape. Unsupported
terminals retain the buffered-write fallback.
The installed default sets `config.ui.plain_prompt = true`, causing the standard
UI to commit one plain startup prompt before reading stdin when interactive mode is
unavailable; other profiles can opt into the same fallback behavior.
Cell measurement uses local wcwidth-style zero-width combining/modifier ranges
and known East Asian wide/emoji ranges; other printable codepoints are one cell.
Each repaint is validated and buffered before one write.

Portability is intentionally conservative: terminal mode uses Zig's POSIX/stdlib
abstractions. Interactive dimensions come from `TIOCGWINSZ`, with
`COLUMNS`/`LINES` and then 80x24 as fallbacks; `SIGWINCH` refreshes runtime cofx
and dispatches `terminal/resize`. Streaming operations and subprocess stdin/
stdout pumping run concurrently, while Lua transactions remain serialized.
Bracketed paste, fragmented UTF-8, Enter, backspace, arrows, Page Up/Down,
Escape, Ctrl-C, and EOF are decoded. Decoder, presenter,
and installed-layout behavior are covered without third-party
test dependencies. The integration suite also exercises resize and cancellation
through a PTY on supported platforms; the optional Python regressions cover
Ghostty input, settled frames, and presentation while the Fennel VM is blocked.
Zig 0.16 no longer
exports `std.posix.read/write/isatty`; misa uses the corresponding
`std.Io.File` portable abstractions. Raw-mode reads are readiness-gated with
VMIN=0/VTIME=0; POSIX signal actions restore terminal modes before chaining normal
termination signals. Bracketed-paste mode is enabled while the managed screen is active.

## Standard extensions

Features own their behavior and presentation together. For example,
`misa.editor.render`, `misa.dialogs.render`, and `misa.transcript.tools.render`
implement feature-specific visual roles beside the models they present.
`misa.markdown` owns parsing and its `misa.markdown.render` terminal renderer;
choice matching belongs to `misa.choices.matching`. Shared input types or a common
component interface do not make these features one subsystem.

`misa.ui.components` resolves visual roles to registered implementations and
protects component boundaries. Its children provide shared content, truncation,
data, and button primitives. `misa.ui.layout` supplies terminal-cell width,
fitting, semantic-span wrapping, and responsive-column operations. Root chrome,
status composition, themes, and animations also live under `misa.ui`.
Theme resolution is centralized at the component registry boundary. Custom code
calls `misa.components.render(db, role, model, context)` and
`misa.animations.span(db, role, options?)` for clock-driven visual motion.
Component `render(model, context, previous?)` receives those tables directly and must not
mutate them or any nested values. Allocate output records when decorating input
data. Syntax projections in message models are ordinary immutable tables, not
callbacks; their document/capture identities survive the component boundary.
For a retained collection, use
`misa.components.project(db, owner_id, [{id, role, model}, ...], context)`;
its `views` vector follows item order. Owner and item IDs must be nonempty strings,
with unique item IDs per collection. Unchanged model/context fields reuse output;
theme and relevant hover changes resolve decoration without recomputing semantics.
Components may return an immutable incremental hint as a second result, received
as `previous` on the next semantic computation. Rendering must remain correct
without a hint. The framework's subscription scope owns these values and rolls
them back with rejected transactions. Direct `components.render` is uncached.
Components registered with `compose=true` receive `context.render_child(role,
model, child_context?)` for nested semantic views. Children use the same role
selections; swapping a child invalidates its composing parent's cached output.
Theme resolution happens once after composition. Leaf components receive the
original context directly.
Prepare subscription-derived model data before calling the collection; retained
component callbacks consume model/context and cannot recursively query subscriptions.
Configure individual roles with
`config.components.roles` (for example, `"picker": "my.picker"`) and dispatch
`components/swap` with `role` and `implementation` to swap one at runtime.
`misa.ui.themes`/`misa.ui.themes.default` and `misa.ui.animations`/`misa.ui.animations.default` are independent
data registries switched with `themes/swap` and `animations/swap`. A theme is
`{palette={name=<default|ANSI|RGB>},styles={token=<style record>}}`; palette
names may be used as style foregrounds or backgrounds. A component span names one semantic
token or an ordered list such as `{"assistant","bold"}`. Theme resolution
merges those records at the component boundary, so role color and independent
attributes compose without predeclared combination tokens. `plain`, `label`,
`value`, and `keybinding` are required standard tokens. The default also
provides distinct `rail.user`, `rail.assistant`, `rail.thinking`, `rail.tool`,
and `rail.error` tokens. Components may set `surface="surface.assistant"`
on a rendered result or individual line: the registry applies that style under
every span and fills the row to the available width. The default `surface.*`
tokens provide muted backgrounds; body text uses the terminal's default foreground.
Animation roles are selected with `config.animations.roles`. Pure visual motion
uses `misa.animations.span(db, role, options?)`, which returns a semantic span with
an animation descriptor. Options include a stable `id` (default
`"animation/" .. role`), an initial `phase` (default 0), and a semantic `style`.
Use distinct stable IDs for independent instances, or share an ID and vary
`phase` for synchronized motion such as a moving highlight. The
terminal owns the monotonic clock, preserves an ID's epoch across publications,
and repaints only changed animation slots. These updates do not invoke Fennel,
change `db`, or produce `animations/tick` events; visual motion continues while
the session processes an event.

The default activity indicator animates only its fixed-width dot pulse every
160 ms while the agent is working; the mode label remains ordinary text. Idle
and single-frame animations schedule no visual updates. Set
`config.animations.enabled=false` for a static indicator, or adjust
`config.animations.interval_ms`. Custom animations register `frames` and an
optional `still` frame for disabled motion; the built-in `static` animation can
also be selected per role. Reduced motion returns the still frame without an
animation descriptor.

A native span can declare `animation={id,interval_ms,phase?,frames={...}}` directly.
Each frame supplies optional `text` and `style`: omitted text uses the base
span's text, and style fields override the base style while preserving its link.
Styles at this boundary are resolved native style records. Every frame must
occupy the same positive terminal-cell width and preserve grapheme boundaries.
Descriptors allow at most 64 frames, IDs at most 256 bytes, and a semantic view
at most 128 visible animated slots with 1 MiB of compiled payloads. Partially
clipped spans retain their fallback text and do not schedule animation updates.
Headless output uses the base span text without ANSI animation output.

Stateful behavior still uses ordinary events and effects. Plugins that need
transactional ticks can explicitly dispatch `animations/start` and
`animations/stop`, handle `animations/tick`, and call
`misa.animations.frame(db, role, tick?)`; these retain the `timer/start` and
`timer/stop` path. Default visual activity does not start those timers.
Selections live in `db`, so failed transactions roll back; successful swaps persist through
the generic state service. Set `persist = false` in the corresponding config
section to disable persistence. These registries contain no input or agent behavior.

`misa.transcript` owns ordered response and block models in `db.messages.responses` and
`db.messages.blocks` for user, assistant, thinking, tool call/result,
authentication, and harness entries. The root managed view reprojects those
models. Stable `transcript/response-*` and `transcript/block-*` lifecycle events
append stream chunks without repeatedly copying accumulated responses, then compact
each block once at finalization. A response owns one contiguous window of
`db.messages.blocks`, so a standalone message that arrives while a response is
still streaming — a harness notice, a released turn, an unmatched tool result — is
ordered after that response's blocks instead of splitting its window. Thinking and
active assistant blocks
carry pending indicators. Tool calls remain one correlated section from pending
through success, error, or cancellation: their result updates the matching call
in place, while unmatched custom results may fall back to a standalone section.
Interruption marks
visible partial blocks without promoting them to provider history. The `:transcript.detail` action
or the configurable global `alt+t` binding reprojects the transcript with
details. Page Up / Page Down (also `alt+k` / `alt+j`) scroll while input and a
responsive semantic indicator row remain visible below it. Consecutive assistant
responses and their tools share one turn footer, with a blank line between blocks.
The footer shows the turn's start time, duration, and combined cost. A single-request
turn also shows its provider throughput; multi-request turns omit that per-request
rate. Duration spans request dispatch through the latest tool completion; parallel
tool durations are not summed. Tool headings show execution durations and primary
paths; shell commands appear in highlighted code blocks. Completed tools use their
background colors without redundant success/error labels. Compact errors show a
short explanation; verbose mode reveals the full diagnostics. Core file errors
include actionable messages, with their native error codes in the details.
Every row with the `┃` left rail shares its block background, including headings,
borders, spacing, and dim omitted-line notices. Group dividers have blank lines
around them and stay outside those backgrounds. User and thinking blocks rely on
their distinct styling rather than redundant labels. Collapsed thinking shows a
short excerpt of its actual text and a hidden-line count. Markdown
quotes use the thinner `▏` rail. The focused `misa.markdown`
extension performs a pure parse into semantic blocks and inlines; `misa.markdown.render` turns that data into
terminal flow, while `misa.transcript.render` supplies the outer rail, block surface, and collapsed previews.
`misa.transcript.tools.render` composes compact lifecycle headings and reusable content views;
tool descriptions remain API documentation and are not displayed in the transcript.
`misa.transcript.tools` binds tools to generic field, text, code, numbered-line, and
diff components, with a generic fallback for unconfigured tools. These bindings
live outside tool definitions; content components know nothing about tool names
or execution. `content.truncated` bounds rendered rows and shows a dim exact count
of hidden lines. Selecting content exposes its canonical source, without copying
headings, margins, or truncation notices. Rendering includes visibly graded
streaming headings, composable emphasis, Unicode task checkboxes, nested lists and continuations, thematic rules,
quotes, responsive bordered tables, inline/fenced code, and OSC 8-capable links.
Fenced code has no frame or language-label rows. Line numbers, code, and trailing
padding share `surface.code`; three columns on each side keep the parent block's
background. Narrow layouts reduce the margins and gutter to preserve readable code.
Languages still select the `misa.transcript.syntax` extension's asynchronous captures when their grammar is installed.
Code renders plainly while highlighting is pending; stale streaming results are
discarded and the latest source is requested. Rendering consumes capture data
without loading grammars or calling a native parser. The pure `misa.ui.layout`
service uses the same wcwidth-style combining, modifier, East Asian wide, and
emoji ranges as the native presenter. It also owns editor grapheme boundaries,
so combining sequences, emoji ZWJ sequences, and virama-attached marks are never
split by Left, Right, Backspace, or width wrapping. The editor input component
maps its logical UTF-8 cursor to a prompt-prefixed physical wrapped row/byte;
root composition windows those rows around the cursor, including on narrow
terminals and across explicit newlines. Message spans and picker options likewise
wrap on grapheme/UTF-8 boundaries using terminal cells rather than bytes or scalar count.
Set `config.messages.markdown` to `false` (or `plain` to `true`) for literal text.
Markdown has no source-byte, block-count, inline-count, depth, or link-length
cutoff, and long messages retain their formatting. Assistant, user, and tool-result
text is retained in full, including streaming chunks. `config.messages` controls
initial `verbose` and the structured tool-argument `redact_keys`, `max_string`,
`max_items`, and `max_depth` limits. Headless output remains plain.

The stock presentation `elements` fragment supplies shared content components;
its `tools` and `transcript` fragments supply the matching policy and group wiring.
Presentation extensions can register a tool binding without modifying its definition:

```fennel
{:tool-presentations
 {:query {:subject :database :fields [:database :sql]
          :code :sql :language :sql :numbered false}}}
```

A binding may instead be a pure function returning
`{:arguments [{:role :content.fields :model {...}}] :result {:role :content.text :model {...}}}`.
The generic roles accept ordinary content: `content.fields` takes labeled values,
`content.lines` takes numbered rows with optional source offsets, and
`content.truncated` takes rendered lines and a visible-line limit. Applications
choose the bindings; renderers never inspect tool definitions or execute tools.
For streaming, `misa.markdown.new-document():update(text)` retains completed
blocks and reparses the final two blocks, where appended syntax can change the
interpretation. Replacements rebuild the document; unchanged normalized source
reuses it. The unfinished block can still reflow, and a large unfinished fence,
table, or paragraph is reparsed as a unit. Ordinary inline text is scanned in runs;
terminator searches retain their next match or failure so malformed openers do
not repeatedly search the same suffix. Quote prefixes and highlighted code lines
are scanned by source offset. Deep list/quote indentation is fitted to the
viewport without changing the parsed depth or hiding the body.
`misa.markdown.view.project(text, options, previous?)` returns an immutable
projection containing `document`, `entries`, and `lines`. Pass a prior projection
explicitly for incremental parsing and per-block layout reuse, keyed by
parsed-block identity, capture identity, width, and semantic styles.
Explicit `options.document` and `options.captures` are immutable inputs; changing
either invalidates document-level reuse. Revision counters are not layout inputs.
Unchanged inputs return the same projection; branches never modify the previous
projection. The former mutable Markdown-view object is removed. Default message
components return projections explicitly as incremental hints and copy cached
bodies before applying titles and rails. The component collection owns those
hints and themed output; there is no global message-document cache.
Animation-only redraws therefore reuse parsing, highlighting, and body layout.
`misa.agent` emits explicit stable response/block start, delta, end, and interruption
`transcript/*` events plus `agent/status` and `agent/usage`; visual extensions do
not inspect or reconstruct shadows from its private orchestration state. UI cancellation
dispatches the semantic `agent/cancel-active` action, and the agent alone resolves
that action to its active provider operation or every pending native tool call ID.
Cancellation intent remains set while completions are drained, prevents model
continuation, then returns the harness to ready with visible cancelled/interrupted
transcript state. Providers
normalize every response to `agent/stream-start`, `agent/stream-delta`, optional
`agent/stream-usage`, and `agent/stream-end`/`agent/stream-error`. Deltas cover
text, thinking, and incrementally assembled tool calls. Protocol adapters use
`misa.stream.effects` to combine adjacent plain text or thinking deltas
already present in one transport batch. Request changes, different delta kinds,
extra metadata, and intervening effects preserve their ordering boundaries;
there is no waiting for more chunks. This avoids a full event/view transaction
for every token while retaining validation before each committed event.
Finalized tool argument
chunks are compacted once before the transcript is reprojected. The focused `misa.json`
extension supplies `misa.json.decode` and `misa.json.encode` at protocol
boundaries. Before continuation, the agent parses every tool call into a
provider-neutral `arguments` table and removes transport JSON; adapters encode
that table only when their wire protocol requires a JSON string. This also
keeps unknown-tool calls replayable after a provider switch. Only the agent correlates
request IDs and promotes a completed assistant response into provider history;
an interrupted response remains marked in the visible transcript but is not
replayed to the provider.

Agent delta assembly is extensible through
`{["agent-deltas"]={["my_delta"]=function(stream, delta, request_id) ... end}}`.
Pure handlers return `{patch=<stream-state patch>, fx=<ordered array>}` or nil.
The agent applies request/cancellation correlation before dispatch. Handlers
assemble canonical text, thinking, or tool-call blocks and emit transcript
events; they do not mutate prior stream state or append conversation history.

Transcript block updates expose
`{["transcript-deltas"]={["block_kind"]=function(block, event, policy) ... end}}`.
Pure reducers return a block patch or nil; the transcript owner applies it without
mutating the event or earlier blocks. `policy` provides the configured preview
limits and redaction keys. This registry handles deltas, not block creation.

The transcript owner publishes `{type="transcript/updated", response_id=...,
block_id=...}` after changing blocks. `block_id` is optional for a whole-response
update. Consumers call `misa.transcript.blocks(db, response_id, block_id?)` for
the current immutable blocks without depending on response indices or storage
layout. Missing targets return an empty array. An explicit notification without
`response_id` requests processing of all blocks, for bulk imports that install
canonical transcript state. Notifications identify what to read, not a copied
text snapshot; queued duplicates safely observe the latest committed source.

`transcript-presentations[block_kind]` is
`function(model, transcript_state, selected_range) ... end`. Pure projectors
return `{role=<component role>, model=<additional render fields>}` or nil to omit
the block. Selection is nil for other blocks. These projections choose rendering
without changing transcript facts or the component's returned line collection.

`misa.ui.status.indicators` is a focused registry for semantic status values. Features return
`indicators[id] = {label?,icon?,hotkey?,query={"query-id",...}}`
in their definition catalogs. Queries use the existing subscription graph and explicit
dependencies. They return nil to omit an item, or a typed fact; false and zero
are values, not omission signals. The callback-based `value` API is removed.
`config.status.indicators` selects order, label/icon representation, an optional
structured keybinding reminder, and drop priority. The common component styles the standard
`label`, `value`, and `keybinding` tokens independently and
removes low-priority items at narrow widths. Standard registrations cover
activity, model, effort (with its cycle hotkey), session usage, context usage,
and transcript detail (`summary`/`verbose`). Root composition stays generic.
The component role is `status.indicators`, including the activity-only fallback
when the registry is omitted. The former `status.metrics` role and `metrics`
model are removed; custom status components should consume `model.indicators`.
Each item carries a `fact`, such as `{type="tokens",value=1234}`,
`{type="ratio",used=1234,limit=200000,unit="tokens"}`, or
`{type="percent",value=75,basis="remaining"}`. Facts contain no styles, spans,
formatted suffixes, or animation frames. Built-in types also include `text`,
`boolean`, `number`, `activity` (with `state`), `unavailable` (with `reason`), and
`money` (with `amount`, `currency`, `estimated`, and `unknown`).

`misa.ui.values` implements an open, pure value-rendering dispatcher. Select
`misa.standard.presentation.values` with status or transcript consumers. Applications
add `value-renderers["my-type"] = function(fact,context) ... end` returning semantic spans. `misa.values.render(fact,context?)` invokes it;
unknown types fail explicitly. Replace a renderer by assigning its entry in
`app.definitions["value-renderers"]`. The default component
uses shared compact-number, percentage, and currency formatters, while owning
styles and width dropping. Response cost metadata is a money fact too, including
`pending=true` with no amount before completion. Timestamps reach components as
raw `started_wall_ms`; the `timestamp` value renderer formats milliseconds as
UTC time-of-day. The old formatted response `text` and `cost_format` service are removed.
Animation selection is separate presentation data from
`misa.animations.state(db,role)` / `[:animations/presentation role]`; the
activity renderer turns it into clock-driven spans without changing activity facts.

`misa.models.options` derives request readiness and selected option values entirely
from the active model's `api.request_options` metadata. Required options without
values block a request before the user turn is recorded and emit a structured
harness problem. `misa.models.effort` is the reasoning-effort affordance: `/effort` lists
only the selected model's declared choices, and the configurable global
`cycle_effort` binding (default `alt+f`) cycles those choices. Model switches
retain an equivalent value when supported, otherwise use the new model default;
models without reasoning support simply expose no effort value.

`misa.choices` owns generic choice state and narrowing transitions shared by inline
completion and overlays. The item contract separates stable `id`, emitted
`value`, semantic `display`, hidden `search`, optional `preview`, and `path`.
Choice sources are registered with `{["choice-sources"]={[id]=source}}`; any item can narrow
to another source, and empty-query Backspace pops the whole narrowing frame.
Selected rows retain the standard selected style in every panel; only the
active leftmost panel receives the `>` focus marker.

Choice views are immutable implementations registered with
`{["choice-views"]={[id]=view}}`. The built-ins are `all`, `favorites`, `frecency`, and
`browse`. Browse shows up to three recent choices above the remaining choices;
searching produces a single ranked list. `config.choices.recent_limit` adjusts
that count. Model and argument pickers use Browse beside Favorites by default.

Choice sessions are immutable data. `choices.refresh`, `choices.set-items`, and
`choices.replace-view` return a new session; callers must retain that return value.
`choices.input` and `choices.accept` return a result containing `session` alongside
outcomes such as `accepted`, `cancelled`, and `narrowed`. Rows and layout never
mutate the supplied session. Add input transitions with
`{["choice-inputs"]={["my_action"]=function(session, event, db) ... end}}`;
the handler returns the same result shape and must leave its inputs unchanged.

Editor handlers also return immutable patches. Register an additional text-edit
kind with `{["editor-edits"]={["my_edit"]=function(editor, event) ... end}}`.
It returns the next editor data, without mutating either input; slash-choice
synchronization then runs over that result. Submission and modal choice outcomes
remain separate from these text edits.

The Vim policy accepts the `editing-motions` and `editing-actions`
catalogs, keyed by motion or action ID. A motion receives editor data and returns a
grapheme-boundary byte offset. An action receives `(editor, editing, event, db)`
and returns `{editor=<patch>, editing=<patch>, fx=<array>}`. Both are pure;
the policy handles undo bookkeeping and selection projection around the result.
Editor-owned transitions call the optional pure
`editor.transition(editing, previous_editor, next_editor, reason, interactive)`
service in the same transaction. It returns the next editing state; reasons are
`insert`, `edit`, `discard`, `restore`, `steer`, `undo`, `redo`, and `preserve`.
Undo bookkeeping does not inspect unrelated events or infer submission from a
global before/after snapshot. Modal input uses event-scoped routes. Leaving modal
editing through Ctrl-C, Ctrl-D, or EOF resets navigation in an explicit handler,
then queues the original input for normal editor handling.

Input routing priorities are dialog capture (1000), a picker's action-palette
shortcut (900), picker capture (800), global action declarations (700), transcript
selection (500), transcript scrolling (400), history (300), and modal editing
(200). Unclaimed input reaches the editor. Model, effort, and command-picker
shortcuts are action data, not feature-specific routing callbacks. Keybinding
collisions within a context report an error instead of choosing by registration
order. Raw Alt events remain intact; keybinding lookup recognizes them directly.

`config.choices.purposes` maps a purpose to ordered views; Right Arrow rotates
the active view to the left. Extensions can supply their own projections and
section labels, replace individual components, or replace the picker entirely.
Overlays use the terminal width with two cells of left/right padding by default;
`config.choices.overlay` supports padding and optional preferred/min/max bounds.
A visible range reports hidden results. Wrapped rows and their hotkeys share
one measured projection. A choice taller than the available space keeps a
selectable representation with an explicit ellipsis. Input docks participate in
inline geometry, so positional hotkeys refer to the rows actually displayed.

Inline and overlay sessions resolve the same choice keybinding declarations and
positional banks. `misa.choices.layout` is the single projection for responsive
preferred/min/max overlay bounds, preview and panel allocation, shared hints,
and positional targets. The picker component only renders that projection.
Select `misa.standard.keybindings` with choice layout and the editor, picker,
and status components. All key hints use its shared token renderer, preserving input case;
equivalent encodings such as `ctrl_n` and `ctrl+n` produce identical spans.
Select `misa.standard.choices.preview` with choice layout. The
`misa.choices.preview` implementation renders typed preview data to
semantic lines before geometry is calculated; compact and overlay input handling
use the same resulting line heights and targets. Extensions register
`choice-previews["my-preview"] = function(model,context) ... end`, returning semantic lines. `context` supplies `columns` and `compact`.
The preview type indexes `app.definitions["choice-previews"]` directly.
Assign a different function at that key to replace its renderer; unknown types
fail explicitly.

Model previews carry `type="model"`, raw `context_window`, and `cost` facts from
`misa.costs.model`: `currency`, `token_unit`, `pricing`, `estimated`, and
`unavailable`. Rate numbers are not converted to strings by model/cost owners.
`misa.models.preview` contributes the model preview renderer; it owns summary
wording, currency precision, and layout. The choice preview module owns dispatch
and generic metadata presentation.
The picker receives both `preview.model` and rendered `preview.lines` records;
custom previews retain their action, link, and animation span metadata.
Overlays remain bounded, nonexclusive regions of the managed root: query input
comes first, then semantic preview, panels, and the shared key reference below
the panels. All key hints use structured tokens and render Alt as `⌥`.
Visible choices start with right-hand home keys `Alt-J/K/L/H/;`. Further
choices use right–left sequences such as `Alt-U F`, `Alt-I D`, and `Alt-O S`,
then right–left–right sequences such as `Alt-U R J`. Home keys finish a
sequence; nearby keys continue it. This distributes shortcuts across multiple
starting keys, alternating hands after the initial Alt chord. The primary panel
gets the five single-key shortcuts; remaining positions interleave across panels.
There are 25 two-key and 125 three-key combinations before another key is needed.
Every visible row gets a stable, prefix-free shortcut. The entered prefix is
highlighted without changing the row text or wrapping. A pending shortcut is
bound to its original visible item; a resize or catalogue update cannot silently
retarget it. While waiting, other bindings and clickable hints are disabled;
Escape, Backspace, or an unmatched key aborts without editing the query.
Tab extends the common candidate path; when it cannot extend further, it completes
the highlighted choice. Tab never submits.
`open_overlay` remains configurable but has no default binding. `replace_view`
(default `alt+/`) opens the nested `picker-picker`; replacing the active view is
kept only for that choice session and never changes purpose defaults.

`misa.dialogs` owns correlated modal/progress/alert lifecycle, actions, cancellation,
and optional text input. Buttons activate by click or their configurable binding;
there is no implicit first action. Only an action with `primary=true` responds to
Enter. A button may declare `confirm={title,message,label}` to require the shared
confirmation flow, and `persistent=true` to keep the original dialog open after
completion. Escape cancels a confirmation first, then dismisses the dialog.

Dialogs may carry `sections=[{id?,title?,heading?,rows=[{label,fact,meter?,detail?,actions?}]}]`.
Facts and details use `misa.ui.values`' typed formatting; optional meters carry `used` and
`limit`. Row action IDs refer to the dialog's action definitions, with `inline=true`
placing their buttons in the row. `misa.ui.components.data` measures shared columns across
all sections, and `misa.ui.components.buttons` uses the ordinary keybinding and action span
presentation. `misa.dialogs.view` composes data with the replaceable dialog chrome.
`dialog/update` replaces supplied sections as a collection while preserving open
confirmations and scroll position. Arrow, wheel, and page keys scroll overflow.
Usage supplies this semantic data contract and owns no rendering or input model.
Dialog handlers return patches. Additional ordinary input kinds use
`{["dialog-inputs"]={["my_input"]=function(dialog, event) ... end}}`;
the pure handler returns `{patch=<application patch>, fx=<array>}`. Protected
dialogs bypass ordinary input handlers entirely.
A protected dialog starts `input/protected` for a waiting native operation and
correlation. Its bounded 64 KiB buffer stays native; Lua receives only length,
submission/cancellation, and capacity-error metadata. Buffered input is held
until capture is installed and scrubbed if the operation fails or is cancelled,
so a pasted key cannot fall through into a conversation.
`misa.dialogs.view` projects that state through the replaceable
`dialog` component role. Dialog data and hints are generic—providers do not own
UI paths or rendering. Default dialogs are compact overlays that retain the
transcript, with clickable URLs and wrapped input. `misa.choices.picker` is only the overlay lifecycle adapter and
`misa.choices.picker.view` renders the centralized projection. `misa.commands` normalizes every
typed, picked, or replayed command into one canonical invocation and records
recent invocations in the generic `commands` preference scope. Canonical strings
(such as `/model vendor/model`) are also favorite IDs. Usage is recorded once
per invocation, including typed commands; replay and registered commands share
the same identity. `misa.commands.palette` (global `alt+/`, also used by the
slash menu) composes commands with those recents; commands with completion
narrow to argument choices before emitting that same canonical event.
`misa.models` owns the model catalogue, availability, metadata, and selection
policy. `misa.models.preview` owns its choice presentation; `/model` has no
model-specific picker behavior.
Type in the picker to filter provider-qualified IDs, labels, or model names,
then use the arrow keys and Enter to select. Models from providers that are not logged in
are hidden. OpenAI, Anthropic, OpenRouter, and Kimi catalogues are loaded from
their model APIs after successful authentication; Misa does not maintain
fallback lists for those providers. Set a provider's `discover_models`
to `false` and provide an explicit `models` list to keep a fixed catalogue. The
editor discovers registered slash commands and argument candidates, displays
the same semantic choice rows as the overlay, and delegates filtering,
highlight, narrowing, configured hotkeys, and acceptance to its inline
choice session. Login commands
automatically complete authentication providers contributed by enabled
provider plugins; `/model` completes the current dynamic model catalogue. Its live startup banner
shows the selected model, cumulative session tokens, and latest context use.
Interactive transcript rows are managed in a height-bounded viewport rather
than committed to terminal scrollback. `/clear` resets the in-memory conversation,
transcript, and usage. `misa.agent` owns normalized conversation history,
repeated user turns, provider correlation, parallel tool-result collection,
normalized token usage accounting, and automatic continuation after tools.
Parallel results update their transcript sections immediately, but enter provider
history in the original assistant call order; continuation is emitted once after
all calls settle.
HTTP SSE and process JSONL transports expose the same start/data/end lifecycle
and coalesce at most 32 records per queued transaction; records and complete
responses are bounded. Transport workers publish into a bounded native queue,
while the session serializes Lua transactions, redraws incrementally, and
interleaves terminal reads with stream batches. Slow policy handling therefore
applies backpressure instead of growing the session queue, and Ctrl-C can cancel
the active socket or child promptly without invoking Lua from an I/O thread.
`misa.standard.tools.files` declares `read_file`, `list_directory`, `write_file`,
and `edit_file`; `misa.standard.tools.shell` declares `shell`; and
`misa.standard.tools.web-search` declares `web_search`. Their implementations
live under `misa.tools` and `misa.search`. These are ordinary application choices. `misa mcp` exposes
the application's tool schemas and effect translators as an MCP stdio server. The
Claude provider supplies this bridge through `--mcp-config` whenever tools are
registered, while retaining `--tools ""` so Claude's own tools remain disabled.
The MCP child inherits `MISA_CONFIG`; configurations selected with `--config`
should set `config.providers.claude.mcp_arguments` to
`["mcp", "--config", "/the/same/config.fnl"]`. `mcp_command` defaults to
`misa` and may be set to an absolute executable path.

`web_search` is a single tool over an open `search-backends` registry, so the
agent sees one schema while applications choose how results are obtained.
`config.tools.web_search.backend` (default `codex`) selects the backend, and
`config.tools.web_search.max_results` and `.timeouts` apply to every backend. A
backend's own map overrides those shared settings, for example
`config.tools.web_search.brave = {url = "..."}`. Model providers that expose
the hosted Responses `web_search` tool are backends in their own right: `codex`
reuses the ChatGPT subscription credential and `openai` reuses an OpenAI API
key on their existing Responses endpoints, so a Codex login is already enough.
`brave` and `tavily` call dedicated search APIs and are registered with
`misa login` as API-key providers; `searxng` queries a self-hosted instance and
requires an explicit `url`. Because the wire detail stays in the backend, a
provider or service fits without changing the tool schema or agent policy.
Results enter canonical history, the transcript, the MCP bridge, and the
optional tool summary like any other tool, and a missing credential fails as an
ordinary tool error.

A backend is an entry in `app.definitions["search-backends"]` named
`{build, complete}`. `build(config, arguments, id)` returns exactly one native
effect (any of the fixed effects, so an HTTP API, a process, or a local
transport all fit), and `complete(config, event)` returns
`{text, is_error?}` for that effect's completion. `build` receives the merged
settings for its own backend, so it can read URLs, models, and limits without
knowing the shared `max_results` policy. Register a custom backend without
touching the tool or agent:

```fennel
(tset app.definitions["search-backends"] :my-search
      {:build (fn [config arguments id] ...)
       :complete (fn [config event] ...)})
(tset app.config.tools :web_search {:backend :my-search})
```

The bundled `search-backends` entries validate the same shape at install time.
A Responses-compatible provider backend additionally accepts `model`, `url`,
`instructions`, `tool_choice`, and `tools` overrides under its own config map,
so a different hosted search tool needs no code change.
`misa.providers.fake` keeps its state under
`db.providers.fake`; `misa.providers.command` adapts user executables.
`misa.providers.claude` invokes Claude Code's stream-JSON process protocol and reuses
Claude's existing Pro/Max credentials without copying them into Misa. Its
catalogue uses the current full model IDs from Anthropic's model documentation:
Fable 5.1, Opus 5, Sonnet 5, and Haiku 4.5. At startup Misa reads
`subscriptionType` from `claude auth status`; Max accounts get a 1M Opus
context window and other plans get 200k. To keep a fixed limit, set the model
record's `context_window` and remove the
`app.definitions.events["provider.claude/availability"]` handler. API-backed Anthropic catalogues are
refreshed from `GET /v1/models` for the models available to that API key.
`misa.editor` owns multiline UTF-8 editor state and transitions. `misa.transcript` owns
transcript scrolling and bounded window extraction. `misa.models`, `misa.models.options`,
and `misa.transcript` expose narrow read-only projections used by status and composition;
those consumers never traverse feature-private state. `misa.ui` owns only root
composition. Interactive sessions return to the editor after each response;
explicit argv remains a single headless turn.

`misa.costs` exposes model rates, response totals, and a session status indicator.
`misa.usage` owns captured token usage and quota refresh policy in `db.usage`.
Status consumes its named subscriptions and owns only status display policy.
`misa.usage.dialog` separately contributes the dashboard and `/usage` command;
usage tracking and refresh do not require that presentation module.
`/usage` opens the shared dashboard and dispatches `usage/refresh`; provider
extensions can handle that event independently. Kimi fetches `/usages` from its
configured regional coding API using the stored `kimi-coding` credential. Its
weekly and additional quota windows remain numeric data, including known used,
remaining, limit, and reset values. Results update an open usage dashboard;
closing it does not cancel the request or reopen the dialog on completion.
`config.providers.kimi.usage_url` can override the quota endpoint explicitly.
Providers without usable quota windows are omitted from the dashboard; missing values never imply zero usage.
Concurrent Kimi refresh triggers coalesce into one follow-up request after the
in-flight request finishes, including when it fails.
This integration follows [Kimi Code's usage implementation](https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/ui/shell/usage.py).
The default `plan` status indicator shows the lowest known remaining percentage
across the selected provider's windows; it shows unavailable when fetched data
cannot establish a percentage. Clicking it opens `/usage` and refreshes the data.
It stays hidden when no quota data exists for the selected provider.
Configure it through `config.status.indicators` like other status items.
Quota refresh targets the selected provider when it changes, after authentication
finishes, and after response completion/interruption; streaming tokens do not
trigger quota requests. `/usage` still refreshes all loaded quota providers.
Codex OAuth fetches `https://chatgpt.com/backend-api/wham/usage` using Misa's
stored Codex credential and native account-ID header binding. Main, code-review,
and additional quota windows normalize to percentage facts; their actual window
durations determine labels rather than assuming a fixed primary/secondary order.
The `misa.usage.dialog` extension projects provider facts into the generic `data` component.
Its rows share label and meter columns, with typed values formatted by `misa.ui.values`.
Dates use the local locale/time zone and relative durations, updated once a minute.
Dialog chrome, scrolling, click/hotkey buttons, and confirmations are shared UI
primitives. Usage has no primary action: Enter does nothing until a confirmation
is open. Escape dismisses confirmation first, then closes Usage and resumes input.
Concurrent refreshes coalesce, stale completions are ignored, and failures clear
stale quota values.

Codex reset details come from `/wham/rate-limit-reset-credits`; the dashboard shows
the available count and each returned expiry. The inline **Use reset** button (`r`,
configured by the `usage/codex-reset` keybinding entry) requires confirmation before
POSTing to `/wham/rate-limit-reset-credits/consume`. Pending operations disable the
button; an uncertain result retains the redemption key for an explicit retry.
Refreshing usage never consumes a reset. The API follows the
[Codex reset-credit contract](https://learn.chatgpt.com/docs/app-server#8-earned-rate-limit-resets-chatgpt).
Native credential trust permits only these exact WHAM endpoints, not the surrounding ChatGPT
backend. An explicit `config.providers.openai_codex.usage_url` override still needs
native credential-origin authorization. Account identifiers are not retained in
quota state.

Claude refreshes full plan quota snapshots through the CLI's experimental
`get_usage` control request, introduced in the
[official Agent SDK release](https://github.com/anthropics/claude-agent-sdk-typescript/releases/tag/v0.3.169).
The query sends no prompt, disables hooks and MCP configuration for the probe,
and leaves credential handling inside Claude Code. It reports available quota
windows, model-scoped limits, subscription type and extra-usage amounts. Monetary
scaling uses explicit decimal places when present, otherwise the CLI's currency
minor-unit conversion. Extra usage is one row with enabled state, spending, and
monthly limit. **Manage** (`e`, configured by the `usage/extra-manage` keybinding entry)
opens Claude's usage settings for enablement and limit changes; the CLI has no
exposed settings-update control. `config.links.command` selects the browser opener
(default `["xdg-open"]`; use `["open"]` on macOS). The probe session's cost is not
used as Misa's conversation cost.
Refreshes coalesce; stale completions are ignored. Unsupported CLI versions,
failed queries and missing quota data show unavailable instead of invented limits.
The experimental response may change; CLI 2.1.261 was checked against a live account.
`providers.claude.executable` selects the CLI, and `usage_timeouts` can override
the bounded probe's startup/idle/overall millisecond timeouts (10s/10s/30s).

Claude also consumes `rate_limit_event` records through its existing stream.
A named window updates its prior observation without erasing other fetched
windows; missing utilization clears that window's percentage. An `allowed`
status does not imply zero usage. The stream wire fields and fractional utilization follow the
[official SDK parser](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/message_parser.py)
and [rate-limit types](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/types.py).
Provider-reported USD takes precedence over estimates; missing prices remain
explicitly unknown. Dynamic provider metadata supplies prices where available.
`config.costs.models["provider/model-id"]` overrides `input`, `output`,
`cache_read`, and `cache_write` rates in USD per million tokens, plus optional
`request` in USD per request. A response snapshots its model rates when it
starts. Selectors always display canonical `provider/model-id`; friendly model
names remain searchable. Enter and Space after `/model` enter the same inline
argument choices, and accepting a model executes the command.

`config.models.catalogue_filter` controls provider-neutral browse curation:
`max_age_days` defaults to 365, `popular_limit` to 30, and `enabled=false`
shows everything. Providers can supply `created` (Unix seconds), `recommended`,
and `popularity_rank`; unknown ages remain visible. Selected models, favorites,
recently used models, and provider recommendations remain visible regardless of age.
Search and the All view retain the full available catalogue. No popularity score
is inferred from a model name or price.

Misa uses one selected model for every request. Choose it with `/model
provider/model` or the picker (Alt-M). The choice persists across restarts and is
restored before the session's first request; `config.models.default` supplies the
initial value only when nothing has been selected yet. A saved model whose
provider registers its catalogue later stays preferred and becomes selected when
that catalogue arrives, so startup neither substitutes the configured default for
a saved choice nor sends a request before the selection is known; a queued prompt
waits for it. Extensions read the resolved model from `db.models.selected` (the
`models/selected` subscription), and automatic compaction runs on the same
selected model.

The default theme inherits terminal body text and base background. Choose
`config.themes.appearance = "light"` for light terminal backgrounds (default
`"dark"`); `themes.palette` overrides named colors using `#rrggbb`, terminal
colors, or RGB records, and `themes.styles` overrides semantic styles. Message
surfaces, rails, accents, syntax colors, and selection are separate tokens.

## Keyboard interaction

The default profile starts in insert mode. Escape enters normal mode; `i`/`a`
resume insertion, `I`/`A` use the start/end of the line, and `o`/`O` open a line.
`h j k l`, `w b e`, `0 $`, and `g G` navigate; `d`/`c`/`y` combine with motions,
with `dd`/`cc`/`yy` selecting a line. `v` selects graphemes and `V` selects lines;
`y` copies, `x` deletes, `p` pastes the internal register, and `u`/`U` undo/redo.
Enter submits; Shift-Enter inserts a newline (including Ghostty CSI-u input).
The prompt shows insert (`┌`), normal (`◆`), or visual (`◇`)
mode, and continuations share a vertical rail. Set `config.editing.mode` to
`"plain"` to disable modal editing, or replace the `misa.editor.editing` extension. Bindings
are data in `app.definitions.keybindings`; change the `default` array of the
corresponding `editor.normal/...` entry.

`misa.editor.history` records accepted submissions, deduplicating consecutive identical
prompts. Ctrl-P/N or Alt-P/N move backward/forward through history; Up/Down do
so at the first/last input line in insert mode. Moving forward past the newest
entry restores the original draft, cursor, and attachments. Ctrl-R opens fuzzy
history search with global deduplication, newest first. Search selection restores
the input for editing. Configure `history.persist`, `history.max_entries`
(default 500), and `history.max_bytes` (default 65536).

You can keep typing while the model responds. Enter appends to one pending
message, separating submissions with newlines; completion sends that message.
Alt-E brings it back into the draft, preserving any existing draft and images.
Alt-Enter interrupts and sends the pending message together with the draft.
Ctrl-C interrupts while preserving the draft. Effort cycling uses Alt-F.
`misa.editor.queue` owns scheduling, `misa.editor.queue.view` owns its dock, and the editor targets the
installed submission capability. These plugins can be replaced independently.
The dock renders the `pending-prompt` component role with
`{pending=<raw text>, attachment_count=<number>}`. Draft attachment controls use
`attachment-controls` with `{pending=<boolean>, count=<number>}`. Their default
components keep existing resting styles and use shared action-hover backgrounds;
override either role through `config.components.roles` to change presentation.

Extensions contribute lifecycle facts through named subscription queries, not
by rewriting completion or input events. Register a subscription and expose its
query through `services["editor.lifecycle.<extension>"] = {"<query-id>"}`.
The editor reads the installed lifecycle catalog after composition. Each query declares its
normal subscription inputs and returns boolean flags: `hold_exit` prevents
one-shot completion from quitting; `block_draft` prevents draft submit/steer
without clearing draft text, attachments, selection, or undo state. Omitted
flags mean false, and contributors combine with logical OR.

The editor checks completion through a queued `editor/completion-check` event,
after the completion transaction commits. Queue reservations and active agent
work both prevent premature exit. Pending image acquisition contributes both
flags, but does not block command, picker, or dialog Enter, nor an already-owned
queue payload. Acquisition completion attaches the image without auto-submitting.
An unsent draft independently prevents exit only in interactive sessions. The
former `agent/completed.keep_alive` event rewrite is no longer consumed.
Queue submission acknowledgement runs behind the immediate events emitted by
`agent/submit`, so an older completion check cannot discard a newer rejection's
diagnostic while releasing its reservation.

Ctrl-V pastes a PNG/JPEG image from the system clipboard; `/image <path>` loads
a file. Ghostty, Kitty, and WezTerm receive bounded inline previews through the
[Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/); other terminals show attachment metadata. Images keep
the original bytes for provider requests. Clipboard I/O and decoding run in
native workers, bounded to 8 MiB compressed, 16 million decoded pixels, and a
480×320 preview. Linux uses `wl-paste` or `xclip`; macOS uses `osascript`.
`config.images.clipboard_command` can replace acquisition with an argv array.
`misa.editor.images` owns acquisition policy, `misa.editor.attachments` composes the draft, and the
`attachment.image` component owns preview geometry. Terminal image caching and
synchronization are separate native mechanisms. Multiplexer image passthrough
is not enabled.

Start input with `:` (no leading whitespace), press F1, or press `:` in normal
mode to open the action palette. It searches registered UI actions and shows
the configured keybinding beside each action. Actions operate on the interface:
editing, transcript navigation/detail, selection, and opening views. Slash
commands operate on the session: model/effort selection, authentication, and
conversation reset. `/` stays discoverable through command and argument
completion; selecting a nonterminal command narrows directly to its arguments.

Alt-S enters structural transcript selection. `h`/`l` move along the rendered
row; `j`/`k` move between rows while preserving the nearest column at the
same structural depth. `J` narrows and `K` widens. A message can narrow to sections, a section's heading
or content, paragraphs, code blocks, tables, rows, cells, lines, words, and
individual graphemes. The selected range is highlighted in the existing rich
transcript; titles, rails, and layout padding keep their resting appearance.
Plain text and rendered Markdown content both carry source markers for range
decoration. A small input dock shows its path and keys. `y` copies its exact
source, including Markdown syntax; Escape returns to the editor. The selected
source is frozen so streaming cannot move the range during navigation. F1 remains
available. The `selection` component role renders the dock independently of
transcript decoration. Page Up/Down, Alt-K/J, and the mouse wheel scroll the
transcript. While browsing, updates below the viewport leave its row position
unchanged; updates above it relocate the top visible content only if displaced.
Scrolling applies its row movement once after that adjustment. At the bottom,
the viewport follows new output until you scroll away.

`misa.clipboard` separates copying from selection. Its default `clipboard/write`
native effect sends OSC 52 to an interactive terminal (up to 1 MiB); terminal
clipboard support must be enabled. Configure `config.clipboard.command` with
an argv array such as `["wl-copy"]`, `["xclip", "-selection", "clipboard"]`, or
`["pbcopy"]` to send copied text to that process's stdin instead. The internal
register always remains available for editor paste.

Stock composition defines actions as `actions[id] = {label,event,binding?,available?}`.
`binding` selects a semantic `{context,action}` keybinding, and `available(db)`
controls contextual discovery. Keyboard mappings live separately in
`definitions.keybindings[id] = {context,action,default=[...]}`. Set that entry's
`default` array to change its keys or `[]` to leave it unbound. An action may
instead carry `keys=[...]` with an optional `context` (default `global`): the
host registers a keybinding for the action under its own ID, which is the same
declaration the separate `keybindings` catalog makes. The `misa.actions`
implementation routes global actions and supplies the palette.
`misa.selection` accepts `{["selection-sources"]={[id]=function(db) ... end}}`
in definition catalogs, with the function returning
source documents with `{id,label,text,kind,first,last,children}` and unique string
IDs across all sources. Ranges are
zero-based, half-open byte offsets. `misa.selection.document` derives semantic
ranges from Markdown and lazily supplies finer ranges; selection policy and
rendering can both be replaced independently. A source may also supply
`layout(db, document, terminal)`, returning its existing rendered rows with
`source_start`/`source_end` byte ranges on content spans. Directional selection
uses those painted coordinates; the transcript supplies this hook through its
normal component renderer. Nonliteral markers retain their original source
range even when their rendered glyph length differs.
Lists expose sibling items and nested lists; an item's range includes its nested
content and continuation lines. Range offsets refer to the original source bytes,
including CRLF line endings.

`v` anchors a visual range and navigation extends it across structural depths
and documents; `v` again returns to the focused node. `selection.ranges(db)`
returns ordered `{id,text,kind,first,last}` slices of the frozen sources.
`selection.state(db, id?)` returns the selected slice for an ID, or the
focused document when no ID is supplied. Copy preserves each slice's original
bytes and inserts a blank line (`\n\n`) between documents. The viewport follows
the focused document, not the beginning of the whole range.
Selection actions are extensible through
`{["selection-actions"]={["my_action"]=function(state, db, event) ... end}}`.
Pure handlers return `{state=<new selection state>, fx=<array>, close=<boolean>}`.
Omitted state is unchanged; `close=true` dismisses selection. Copying is one
action, not a requirement of navigation or range projection.

## Nix

NixOS, nix-darwin, and home-manager select an application through
`programs.misa.configuration`. Configuration files in the Nix store are
world-readable, so they must not contain secrets.

The package and development shell include every grammar from the pinned
nixpkgs using
`pkgs.tree-sitter.withPlugins (_: pkgs.tree-sitter-grammars.allGrammars)`.
The bundle contains parsers but no highlight queries; Misa classifies syntax
node types generically. Fence aliases include common labels such as `js`, `ts`,
`py`, `rb`, `rs`, `sh`, `c++`, `c#`, `yml`, and `md`.

Outside Nix, install tree-sitter (including its pkg-config metadata) and point
`MISA_TREE_SITTER_DIR` at a directory of `<language>.so` parsers. A default can
instead be compiled with `zig build -Dtree-sitter-dir=/path/to/grammars`.
Grammar libraries must export their conventional `tree_sitter_<language>`
symbol. Conversation persistence also requires SQLite development files
(`sqlite3.h` and its pkg-config metadata), like the other system libraries.

`default.nix` exports the package, overlay, shell, modules, `lib`, and
`standardExtensions`. Nix can package the same configuration file:

```nix
let p = import ./path/to/misa { inherit pkgs; }; in
p.lib.mkMisa {
  configuration = ./config.fnl;
}
```

`standardExtensions` mirrors the source tree. For example,
`standardExtensions.misa.editor.init` is `"misa.editor"`,
`standardExtensions.misa.editor.history` is `"misa.editor.history"`, and
`standardExtensions.misa.markdown.render` is
`"misa.markdown.render"`. The `init` attribute identifies an owner's
entrypoint alongside its child modules; it is not part of the module name.

The Home Manager, NixOS, and Darwin modules expose the same file through
`programs.misa.configuration`. If it imports sibling modules, retain their
whole directory in the closure with `configuration = "${./misa-config}/config.fnl"`.
The file contains the ordinary Fennel composition above; Nix does not add a
separate inheritance or merge language.

## Credentials

Run `misa login openai` or `misa login anthropic` to enter an API key without
terminal echo. `misa login openai-codex` uses OpenAI's device flow for a
ChatGPT subscription, `misa login kimi-coding` uses Kimi's device flow, and
`misa login openrouter` uses OpenRouter's PKCE flow. Device OAuth opens the browser
using `xdg-open` on Linux or `open` on macOS; the managed UI retains the transcript
behind a popup with a clickable URL, code, progress, and cancellation. The URL
remains available if browser launch fails. OpenRouter uses a temporary loopback
callback with state validation and falls back to a pasted-code dialog when
callback setup or browser launch is unavailable. Use
`misa status PROVIDER` to inspect login state without exposing credential data
and `misa logout PROVIDER` to remove it. The `misa.providers.auth` extension provides the same
flows inside the TUI as `/login PROVIDER`, `/status PROVIDER`, `/logout
PROVIDER`, and `/account PROVIDER ACCOUNT`.

A provider may hold several named accounts. `misa login PROVIDER [ACCOUNT]`
and `/login PROVIDER [ACCOUNT]` store one account's credential, replacing any
credential that account already holds; an accountless login replaces the
credential the provider currently uses. `misa select PROVIDER ACCOUNT` and
`/account PROVIDER ACCOUNT` switch which stored account the provider's requests
use, without authenticating again. `misa logout PROVIDER [ACCOUNT]` removes one
account, or every account when none is named, and `misa status PROVIDER
[ACCOUNT]` lists the stored accounts with the selected one marked, or reports
one named account. Account names use 1 to 64 letters, digits, `.`, `_`, or `-`.
A credential stored before accounts existed is the `default` account, which is
also what an accountless login writes. For `claude`, login, status, and logout
delegate to `claude auth`, which owns its one account. Interactive API-key login
uses the ordinary popup with masked native input. CLI login still hands the
terminal to Claude and returns automatically; logout does not need a handoff.
OAuth access credentials are refreshed from their
stored refresh tokens. Credential mutex acquisition is cancellable and refresh
network I/O never holds a mutex. A refreshed token is published with a
process/interprocess-locked compare-and-swap over credential generation, refresh
token, and profile; logout or a newer login always wins and stale refreshes can
never resurrect it. Mutations take a sibling interprocess lock, reload the latest
document, merge one provider, and atomically replace it, so concurrent Misa
processes do not lose unrelated grants. Kimi defaults to the official global `.ai`
profile. Select the `.com` endpoint bundle through authentication data:

```fennel
(tset app.definitions.auth-providers :kimi-coding :profile
      (. (require :misa.providers.kimi) :profiles :mainland))
```

The selected profile and trusted refresh/API endpoint metadata are stored with
the credential so regions cannot silently drift. Credentials are
written atomically with mode `0600` beneath a mode `0700` directory. The
path is `$MISA_AUTH_FILE`, otherwise `$XDG_STATE_HOME/misa/auth.json`, otherwise
`$HOME/.local/state/misa/auth.json`. They never belong in regular or Nix
configuration. Standard credentials are injected only for native trusted
origins. A user may explicitly grant an HTTPS custom origin (host only, no path)
through process configuration, for example
`MISA_CREDENTIAL_ORIGINS='{"openai":["https://gateway.example"]}'`; extensions
cannot grant themselves destinations. `misa login claude` delegates to `claude auth login`; Claude Code
continues to own and refresh its existing subscription credentials.

For a smaller application, import the desired `misa.standard.providers.*` and
`misa.standard.protocols.*` catalog fragments. HTTP adapters also need the stock
JSON and stream services. OpenAI, DeepSeek, and OpenRouter share the OpenAI protocol;
Anthropic and Kimi share the Anthropic protocol. ChatGPT subscription access uses
the independent `misa.standard.providers.openai-codex` fragment. UI and tool
presentation fragments can be selected separately.

Model catalogues and discovery choices are explicit application data:

```fennel
(tset app.definitions.models :openai/private-model
      {:id :openai/private-model :provider :openai
       :model "private-model" :label "Private model" :context_window 128000})
(tset app.definitions.auth-providers :openai :discover_models false)
```

Remove other model entries with `nil` when a fixed catalogue is desired.
`config.providers` holds request options such as URLs, timeouts, and the Claude
executable; model lists, discovery flags, authentication profiles, and fixed
context limits live in their corresponding declaration maps. Provider request
and model-normalization operations are public functions for custom adapters.

DeepSeek is available through `misa login deepseek` or `/login deepseek`. It uses
DeepSeek's OpenAI-compatible streaming chat-completions API, including tool calls,
image input, and reasoning, and discovers its model catalogue from
`https://api.deepseek.com/models`. That endpoint reports identifiers only, so
`misa.providers.deepseek` declares the published facts for known identifiers —
context window, prices, and the reasoning ladder — and merges them onto the
discovered rows. An identifier without published facts keeps its row without
invented limits or prices, so a new release appears in `/model` as soon as the API
lists it. V4.1 Flash (`deepseek-flash`) is the current multimodal release; the
retired `deepseek-v4-flash` and `deepseek-v4-flash-vision-exp` names are served and
billed as V4.1 Flash. `deepseek-v4-pro` remains text-only at its own rates.

DeepSeek's request surface is provider-owned. `/effort` offers `none`, `low`,
`high`, and `max` for the V4 family, where `none` disables thinking mode; the
ladder mirrors the mapping DeepSeek publishes for `minimal`, `medium`, and
`xhigh`, and one ladder covers both Flash and Pro even though third-party
catalogues still list Pro as `high`/`max` only. Requests carry `max_tokens` rather than
`max_completion_tokens`, never carry `tool_choice` (DeepSeek rejects forced choice
while thinking), and replay captured thinking as `reasoning_content`, which DeepSeek
requires when the conversation carries tools. A reasoning turn emits no stream bytes
while the model thinks, so `config.providers.deepseek.timeouts` defaults its
first-byte and idle budgets to five minutes; `config.providers.deepseek.max_tokens`
and the remaining request options work as they do for other providers.

DeepSeek doubles its prices during its published peak hours (01:00–04:00 and
06:00–10:00 UTC, Monday through Friday). Cost accounting resolves that multiplier
when a response starts and snapshots the resulting rates, so a retry or a later
estimate cannot move a turn between price tiers, and model previews state the peak
windows beside the off-peak rates. Any model or `config.costs.models` entry may
declare the same shape:

```fennel
{:input 0.15 :output 0.6 :cache_read 0.003
 :peak {:multiplier 2
        :windows [{:start_hour 1 :end_hour 4} {:start_hour 6 :end_hour 10}]
        :weekdays [2 3 4 5 6]}}
```

Windows are half-open UTC hours that wrap midnight when the start exceeds the end,
and weekday numbers follow `Sunday = 1`.

Groq, Together, Fireworks, xAI, Mistral, Cerebras, DeepInfra, Hugging Face,
NVIDIA, Moonshot, Novita, SiliconFlow, and Venice are also included as API-key
providers. They use their OpenAI-compatible chat-completions and model-listing
endpoints, so `/login PROVIDER` and `/model` work consistently across them.
Where a provider has a known key-management page, the login dialog links to it
before accepting the pasted key. `brave` and `tavily` are API-key providers for
the `web_search` tool rather than model providers: `/login brave` and
`/login tavily` store the same native credentials the search backends inject.

OpenAI-compatible request options are composed from provider fragments instead
of being embedded in the shared chat protocol. All compatible providers accept
their supported generation defaults through `config.providers.PROVIDER.request_options`.
OpenRouter also accepts its routing policy through `config.providers.openrouter.routing`:

```fennel
{:providers {:openrouter
             {:request_options {:temperature 0.2
                                :top_p 0.95
                                :reasoning_effort :high}
              :routing {:only [:anthropic :google]
                        :allow_fallbacks false
                        :require_parameters true
                        :data_collection :deny
                        :sort :throughput
                        :quantizations [:fp8 :bf16]
                        :max_price {:prompt 2 :completion 8}
                        :preferred_min_throughput 80
                        :preferred_max_latency 2}}}}
```

Routing settings become OpenRouter's request-level `provider` object. The
standard generation fragment supports `temperature`, `top_p`, `top_k`, `seed`,
`max_tokens`, penalties, `stop`, `tool_choice`, `parallel_tool_calls`, and
`response_format`; OpenRouter adds its `reasoning_effort` translation. Provider
extensions can compose further fragments without changing
`misa.protocols.openai`.

For an OpenAI-compatible endpoint that is not shipped as a preset, add the
fragment returned by `misa.standard.providers.generic` to your application
definitions. Provider IDs begin with `generic/`; Misa stores the HTTPS base URL
alongside its key and injects that key only for requests below the saved URL.

```fennel
(local generic (require :misa.standard.providers.generic))
(local local-ai (generic.provider "generic/local-ai"
                                  {:api :openai
                                   :base_url "https://llm.example/v1"
                                   :label "Local AI"}))
;; Merge local-ai into application.definitions, then use /login generic/local-ai.
```

The generic module also exports `api-key` and `device-oauth` credential-flow
constructors. `device-oauth` implements RFC 8628-style device authorization,
opens the provider's verification page, shows the user code, polls for the
token, and refreshes it with the same client ID. Supply it through `:auth` when
the provider publishes a public device-flow client:

```fennel
(generic.provider "generic/acme"
                  {:api :openai
                   :base_url "https://api.acme.example/v1"
                   :auth (generic.device-oauth
                          "https://api.acme.example/v1"
                          "https://login.acme.example/oauth/device/code"
                          "https://login.acme.example/oauth/token"
                          "acme-cli-public-client")})
```

Use `(generic.api-key base-url provisioning-url)` to give a custom API-key
provider the same key-provisioning link. New OAuth strategies fit beside these
constructors without changing the provider transport.

OpenAI-compatible delta projections can be extended with
`{["openai-deltas"]={["my_delta"]=function(delta, record) ... end}}`.
Pure projections return arrays of normalized agent deltas and run in deterministic catalog
ID order alongside the built-in text, reasoning, and tool-call projections.
Anthropic-compatible streams expose the `anthropic-records`,
`anthropic-block-starts`, and `anthropic-block-deltas` catalogs, keyed by record
or block type. Pure handlers receive `(state, record, request_id, provider)`
and return `{patch=<stream-state patch>, fx=<array>, finish=<boolean>}`. This keeps
signed provider state separate from visible thinking deltas. Terminal handlers
set `terminal=true` or `failed=true` and request `finish=true`.
Codex record handlers are pure and extensible through
`{["codex-records"]={["record.type"]=function(state, record, request_id) ... end}}`.
Handlers return `{patch=<stream-state patch>, fx=<array>, finish=<boolean>}`.
Completion records mark `terminal=true`; failures mark `failed=true`. Either
stops subsequent records from emitting output for that stream.

Claude CLI records expose the `claude-records` and
`claude-stream-events` catalogs, keyed by record or stream event type. Pure handlers receive
`(state, record, request_id)` and return `{state=<new immutable stream state>,
fx=<array>, finish=<boolean>}`; omitted state is unchanged. Terminal handlers set
`result=true` and request `finish=true`. Tools retain one identity across partial
and final assistant records; visible text fallback is tracked per message/block.

File reads return a `snapshot TAG` header and `LINE#HASH|text` rows. Anchored
reads return at most 2000 lines; `start_line` and `max_lines` select a page while
retaining absolute line anchors and the full-file snapshot. Anchored
`edit_file` calls copy that snapshot plus a `start` anchor and optional inclusive
`end`, with plain replacement lines in `new_text`. Set `position` to `before` or
`after` for insertion at one anchor; the default is `replace`, and empty
replacement text deletes the range. Every edit checks the full snapshot as well
as the line hashes, so changes inside a range cannot silently slip through.
Replacement line endings follow the file's LF or CRLF convention. Source and
replacement text are validated before writing, and the response is prepared
before the atomic replacement. Successful edits return a unified diff with two
lines of surrounding context, followed by fresh snapshot anchors. The transcript
shows the diff through the markdown code-block renderer, with one line-number
gutter: removed lines use old-file numbers; other lines use new-file numbers.
File reads, writes, edit diffs, and shell commands/output share that renderer's
surface, spacing, and wrapping. Collapsed shell output shows its last three
rendered rows, with the omitted-row count above; expanding shows the full output.
Snapshot hashes remain in the
tool result but are hidden in the normal view. Pending replacement snippets are
unnumbered; completed edits show the actual diff instead of repeating the snippet.
Re-read after a successful edit or a stale-snapshot error. An empty file exposes
one empty-line anchor. `edit_file` requires `snapshot`, `start`, and `new_text`;
the legacy `old_text` argument is no longer advertised or accepted. The same native implementation serves direct
file tools and the Claude MCP bridge. This adapts the snapshot validation idea
from [oh-my-pi's hashline editor](https://github.com/can1357/oh-my-pi/blob/main/docs/tools/edit.md),
with a smaller explicit-range contract.

Claude's CLI requires `mcp__misa__` names for its MCP transport and allowlist.
Misa removes that private bridge prefix from observed tool names in canonical
history and the transcript; names belonging to other MCP servers remain intact.

The `misa.compaction` extension (included in the default profile) frees context in
long sessions by replacing canonical history with one handoff summary. `/compact`
(also the `compaction.compact` action) runs a tool-free summarization request on the
selected model, with a cancellable progress dialog; `config.compaction.prompt`
overrides the instructions and typed `/compact` arguments are appended to them.
Automatic compaction requires an available selected model that declares a
`context_window`; otherwise the budget is unknown, no automatic attempt is made,
and `/compact` reports that it needs a model. Only a summary produced for an
unchanged conversation replaces canonical history, and it is installed as a user
handoff message followed by a short assistant continuation so the next request
still alternates roles. The visible transcript and scrollback are never rewritten,
so reading position and selection are preserved. `config.compaction.enabled`
(default `true`) enables automatic compaction, which starts from a ready agent once
the estimated prompt reaches `threshold * (context_window - reserve_tokens)` with at
least `min_messages` messages. The model input is bounded by `max_input_bytes`,
keeping the beginning and the most recent work with an explicit omission marker, and
the accumulated summary by `max_summary_bytes`. An empty summary, a failure, or a
summary no shorter than its input leaves history untouched and reports why. Changing
the selected model or losing its availability cancels an active request, and draft
submission is held while a compaction runs. Usage is recorded through `misa.costs`
like any other request.
