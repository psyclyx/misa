# misa

misa is a small event-driven coding-agent harness built with Zig 0.16,
system LuaJIT, bundled Fennel 1.6.0, and system tree-sitter. Terminal presentation uses Zig; bounded
image decoding uses system libpng and libjpeg-turbo.

No extensions are enabled implicitly: `{}` is valid and produces no output.
The shipped extensions include `models`, `agent`, `auth`, `ui`, protocol adapters,
and fake, command, Claude Code, OpenAI, Anthropic, OpenRouter, and Kimi
providers. `tool.files` and `tool.shell` provide optional coding tools. Select
them through the one ordered `extensions` list. Protocol
adapters precede the API providers that use them:

```json
{
  "extensions": [
    "values",
    "json",
    "protocol.anthropic",
    "provider.anthropic",
    "provider.kimi",
    "protocol.openai",
    "provider.openai",
    "provider.openrouter",
    "provider.openai-codex",
    "provider.claude",
    "auth",
    "tool.files",
    "tool.shell",
    "fuzzy",
    "keybindings",
    "actions",
    "clipboard",
    "links",
    "dialogs",
    "commands",
    "choices",
    "preferences",
    "themes",
    "theme.default",
    "animations",
    "animation.default",
    "components",
    "layout",
    "choice_preview",
    "choice_layout",
    "markdown",
    "selection_document",
    "selection",
    "component.markdown",
    "component.image",
    "indicators",
    "component.tool",
    "component.message",
    "component.editor",
    "component.picker",
    "component.status",
    "component.chrome",
    "component.buttons",
    "component.data",
    "component.dialog",
    "component.selection",
    "dialog_view",
    "messages",
    "status",
    "usage",
    "picker",
    "picker_view",
    "models",
    "costs",
    "omnipicker",
    "request_options",
    "effort",
    "agent",
    "queue",
    "queue_view",
    "editor",
    "images",
    "attachments",
    "history",
    "editing",
    "ui"
  ],
  "config": {
    "models": {
      "default": "claude/claude-sonnet-5"
    }
  }
}
```

```sh
zig build
zig build test
zig build test-integration
zig build test-nix
zig build -Doptimize=ReleaseSafe
zig build run -- --config config/default.json hello
```

Use `nix-shell -A shell` for the pinned development dependencies. `zig build test`
runs native unit tests and the named Zig integration cases in `tests/integration/`.
The suite owns temporary configurations, child processes, deadlines, and output
assertions; extension fixtures live in `tests/integration/fixtures/*.fnl`.
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
`python3 tests/http-cancellation.py zig-out/bin/misa` exercises cancellation,
idle timeouts, and compressed responses through the real HTTP transport using
a local server; build the production executable with `zig build` first.
Pass `--installed` to `tests/ghostty-input.py` to verify the executable's installed
catalog rather than loading extensions from the worktree. This mode uses the
catalog installed alongside `misa-fixture` by `zig build fixture-app`.

Without an override, misa loads the installed `share/misa/default.json`, which
selects all shipped real providers, Claude Code by default, coding tools, model
policy, authentication commands, the agent, and the UI as a useful coding
profile. Run `misa login claude` first.
`--config PATH` takes precedence over `MISA_CONFIG`, which takes precedence over
that installed default. Arguments not consumed by `--config` are exposed as
`cofx.argv`.

Bare standard IDs use `MISA_EXTENSION_DIR` when set, otherwise the absolute
`share/misa/extensions` path compiled from `zig build --prefix`. Relocated or
copied installations must set both `MISA_CONFIG` and `MISA_EXTENSION_DIR`; misa
never discovers its own executable path. Values containing `/` or ending in
`.fnl` or `.lua` are literal custom paths. Standard extensions and the embedded
event framework are written in Fennel. The compiler is embedded in the executable;
loading installed or custom Fennel extensions requires no external compiler.
Lua extensions remain supported by the same VM boundary.
The build requires a host `luajit` executable to translate bundled extensions and
the embedded runtime core to portable Lua source. Installed catalog IDs use generated `.lua` files; explicit
`MISA_EXTENSION_DIR` overrides continue to use `.fnl` sources so development edits
cannot be hidden by stale generated files. The original sources are installed
alongside generated files, whose line layout is correlated to the Fennel source
for diagnostics. The embedded runtime core loads translated Lua at startup;
the embedded compiler remains available for custom Fennel modules.

## Event, coeffect, effect, and view contract

An extension returns `{ setup = function(context) ... end }`. Setup returns
`{fx = {...}}`: an ordered array of registration effects, interpreted before the
next extension's setup runs. There is no `run` phase or imperative `misa.reg_*` API.
For example, a Fennel extension can declare an event handler and a shared service:

```fennel
{:setup (fn [context]
  {:fx [{:type :register/event :name :app/start
         :handler (fn [db event cofx]
                    {:fx [{:type :dispatch
                                 :event {:type :example/ready}}]})}
        {:type :register/service :name :example
         :value {:enabled (. context.config :example)}}]})}
```

Registration effects use these payloads:

- `{type="register/event", name=type, handler=fn}`: handlers run in registration order and
  update canonical application state. A handler receives `(db, event, cofx)` and returns nil or
  `{patch=<map>, fx=<ordered array>}`. Patches recursively merge maps, replace vectors and
  scalar values, preserve identity for untouched branches, and support `(misa.replace value)`
  and `misa.delete`. Returning `{db=<table>}` is rejected, including from Lua extensions.
  Treat input state as immutable; allocate new data or use `misa.patch` for updates.
  An empty table patch is a no-op, including at absent or scalar paths; use
  `misa.replace({})` to clear a collection. Nonempty dense numeric arrays replace
  the collection; sparse or mixed numeric/string patch tables are rejected.
  Replacement data is validated recursively, even when its identity matches
  existing state. Data cannot contain patch controls, cycles, metatables,
  callbacks or non-finite numbers. Unchanged branches retain identity, including
  equal replacement data. `misa.delete` and `misa.replace(nil)` remove a key;
  `misa.json_null` stores an explicit JSON null.
- `{type="register/event-route", value={id,event,priority,context,resolve}}`: declares
  a pure route for one source event type. `context` is a subscription query;
  nil makes the route inactive. `resolve(context,event,cofx)` returns nil to
  decline or a semantic event to handle. Highest-priority claim wins; competing
  claims at that priority are an error, not a registration-order tie-break.
  Priority is a finite integer. The winning event goes directly to ordinary
  handlers in the same transaction; it is not recursively routed. If no route
  claims, the original event is handled. Routes cannot return patches or effects.
  Global before/after interceptors are not supported.
- `{type="register/cofx", name=name, handler=fn}`: derives a policy value. Derivations run in
  registration order; a later derivation may read values installed by earlier
  ones. Setup context also includes `host={executable,config_path}` so subprocess
  bridges can reuse the running binary and explicitly selected configuration.
  Base coeffects always include `config`, `argv`,
  `terminal={interactive=<bool>,images=<bool>,columns=<integer>,lines=<integer>}`, and
  `clock={wall_ms=<Unix epoch milliseconds>,monotonic_ms=<monotonic milliseconds>}`.
  These names are reserved. Native code samples both clocks for every
  transaction; elapsed durations must use `monotonic_ms`, never wall time.
  Configuration and argv are shared immutable inputs by contract.
- `{type="register/fx", name=type, handler=fn}`: translates a Fennel policy effect to one native effect
  or an ordered array of native effects.
- `{type="register/view", handler=fn}`: registers exactly one semantic projection.
- `{type="register/sub", value={id=..., inputs=queries, compute=fn}}` (or `value={id=..., read=fn}`): registers a pure
  query projection. Call it as `misa.sub(db, [id, ...args])`; `inputs` is a vector of query vectors
  (or a function of the requested query when dependencies depend on its arguments),
  and `compute` receives their values. Results are memoized by input identity across view passes
  and within each pass. See [subscription contracts](docs/subscriptions.md) for nullable
  inputs, bounded consumer scopes, and speculative commit/rollback.
- `{type="register/view-layer", id=id, handler=fn}`: contributes an optional semantic overlay layer;
  the highest `priority` wins (default 0), allowing temporary palettes above
  input docks for selection and attachments, and modal dialogs;
  the UI composes layers without knowing plugin-owned state.
- `{type="register/model", value=model}`: adds a provider-owned catalogue entry, including an
  optional `context_window` and API metadata. `api.request_options` maps generic
  option names to `{choices={...},default=...,required=?}` declarations and
  `api.request_options_serializer` identifies the provider transport serializer.
- `{type="register/auth-provider", value=provider}`: declares an authentication ID, model
  provider, native `strategy`, and (for OAuth) trusted endpoint `profile` for
  completion and availability tracking. Native validation rejects drift.
  `misa.auth_provider(id)` and `misa.auth_provider_for_model(id)` look up declarations
  in the live registry, including providers registered later during setup.
- `{type="register/command", value={name,description,event,completion?,complete?}}`: adds a
  generic slash command. `completion` names a shared static candidate group;
  `complete(prefix,db)` supplies dynamic candidates when needed.
  `misa.command_invocation(text)` returns a `commands/invoke` event. Its handler
  normalizes arguments, records preferences, and dispatches the declared execution
  event; unrelated events carrying a `command` field are not intercepted.
  Optional `choice_available(db)` and `choice_unavailable` (an event name) declare
  how opening an unavailable command choice is handled.
- `{type="register/completion", group=group, value={value,label?,description?}}`: lets any plugin add
  a candidate to a shared completion group. The UI handles filtering, sorting,
  display, and insertion, so providers only declare their authentication ID.
- `{type="register/tool", value=tool}`: adds a semantic tool schema and its effect type.
  `misa.models()`, `misa.model(id)`, `misa.auth_providers()`, `misa.commands()`, `misa.command(name)`,
  `misa.tools()`, and `misa.tool(name)` expose the sealed registries.
- `{type="register/service", name="namespace.member", value=value}` exports a
  shared service; later extensions can consume it through `misa.namespace.member`.
- `{type="register/setup-effect", name=type, handler=fn}` declares a plugin-owned
  setup effect interpreter, such as the component and theme registries. Its name
  must use the `register/` namespace. Its
  handler receives the effect and may return `{fx={...}}` to expand it into
  further setup effects.

Setup accepts registration effects only. Event transactions accept runtime effects
only: registrations cannot be emitted by event handlers or runtime effect
translators. Registrations seal after all extensions have completed setup.
Perform startup IO through effects from an `app/start` handler. Plugin setup must
return service declarations rather than assigning fields directly on `misa`.
Initialize domain state in its owning `app/start` handler, not a global
interceptor. A consumer that needs other startup owners' state should emit an
explicit continuation event: dispatched effects run after the transaction
commits. Bundled `agent/startup` checks settled authentication readiness;
`request-options/reconcile` reads settled model state after startup and model
changes. This avoids depending on extension registration order. Interceptors
remain available for handler plumbing and ordered input routing, not as the
default home for domain event policies.
Protocol factories such as `misa.protocols.openai(spec)` return `{fx={...}}` for
composition into a provider extension's setup result. The asynchronous
`syntax/highlight` effect accepts `id`, `language`, `source`, and `completion`,
with an optional `timeout_ms` (1–60000; default 1000). Its completion event has
`id`, `ok`, and `data` containing ordered
`{start_byte=<zero-based>, end_byte=<exclusive>, capture=<semantic name>}`
ranges. Captures use a finite generic vocabulary (`comment`, `string`, `number`,
`keyword`, `type`, `function`, `constant`, `variable`, `property`, `tag`,
`attribute`, `operator`, `punctuation`, `escape`, and `embedded`). Highlighting
is derived data: the `syntax` extension tracks pending requests and accepted
revisions, immutable parsed documents, and accepted capture arrays in transactional
state. It consumes explicit `transcript/updated` notifications, not a global
before/after interceptor; projections never schedule highlighting.
The `syntax/projections` subscription incrementally projects document
entries, retaining unchanged results. Point queries use `syntax/projection`;
collection consumers call `misa.syntax_projections(db)` once and pass that
immutable snapshot to `misa.syntax_projection(snapshot, model)`. The latter is a
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

The fixed native effects are:

- `{type="dispatch", event=<table>}`
- `{type="syntax/highlight", language=..., source=..., completion=..., id=..., timeout_ms=?}`
- `{type="process/run", argv={<strings>}, completion=<event type>, id=<string>, timeouts={startup_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="provider/process", argv={<strings>}, completion=<event type>, id=<string>, timeouts={startup_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="http/request", url=..., json=..., credential=..., completion=..., id=..., timeouts={first_byte_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="file/read", path=..., completion=..., id=...}`
- `{type="file/list", path=..., completion=..., id=...}`
- `{type="file/write", path=..., content=..., completion=..., id=...}`
- `{type="file/edit", path=..., content=..., replacement=..., completion=..., id=...}`
- `{type="json/decode", source=..., completion=..., id=...}`
- `{type="terminal/read"}`
- `{type="clipboard/write", text=<up to 1 MiB>}`
- `{type="image/load", path=..., id=..., completion=...}`
- `{type="image/paste", argv=<optional clipboard command>, id=..., completion=...}`
- `{type="input/protected", id=..., correlation=..., completion=...}`
- `{type="timer/start", interval_ms=<10..60000>, completion=..., id=...}` / `{type="timer/stop", id=...}`
- `{type="operation/cancel", id=...}`
- `{type="auth/command", action=..., provider=..., strategy=..., profile=...,
completion=..., interaction=..., id=...}` / `{type="auth/respond", id=...,
correlation=..., action=..., value=...}`
- `{type="state/load", namespace=..., completion=...}` / `{type="state/save", namespace=..., data=...}`
- `{type="view/commit", lines=<semantic lines>}`
- `{type="app/quit"}`

Unknown native effects fail the session. `http/request` injects credentials by
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
state. Fennel handlers and semantic view projection share one serialized VM.
The session validates effects and views before committing state, then transfers
an owned view after the synchronous dispatch chain settles.

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
`terminal/presenter.zig`, and `terminal/width.zig` own their respective mechanisms.

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

The application UI is split at semantic boundaries. `components` resolves
visual roles to registered implementations. The independently loadable
`component.message`, `component.group`, `component.tool`, `component.content`,
`component.truncation`, `component.markdown`, `component.editor`, `component.picker`, `component.status`,
`component.chrome`, and `component.dialog` plugins provide the default roles; none owns behavior or
depends on a theme. `layout` provides pure terminal-cell width, fitting,
semantic-span wrapping, and responsive-column primitives. Theme resolution is
centralized at the component registry boundary. Custom code calls
`misa.render_component(db, role, model, context)` and
`misa.animation_span(db, role, options?)` for clock-driven visual motion.
Component `render(model, context, previous?)` receives those tables directly and must not
mutate them or any nested values. Allocate output records when decorating input
data. Syntax projections in message models are ordinary immutable tables, not
callbacks; their document/capture identities survive the component boundary.
For a retained collection, use
`misa.project_components(db, owner_id, [{id, role, model}, ...], context)`;
its `views` vector follows item order. Owner and item IDs must be nonempty strings,
with unique item IDs per collection. Unchanged model/context fields reuse output;
theme and relevant hover changes resolve decoration without recomputing semantics.
Components may return an immutable incremental hint as a second result, received
as `previous` on the next semantic computation. Rendering must remain correct
without a hint. The framework's subscription scope owns these values and rolls
them back with rejected transactions. Direct `render_component` is uncached.
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
`themes`/`theme.default` and `animations`/`animation.default` are independent
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
uses `misa.animation_span(db, role, options?)`, which returns a semantic span with
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
`misa.animation_frame(db, role, tick?)`; these retain the `timer/start` and
`timer/stop` path. Default visual activity does not start those timers.
Selections live in `db`, so failed transactions roll back; successful swaps persist through
the generic state service. Set `persist = false` in the corresponding config
section to disable persistence. These registries contain no input or agent behavior.

`messages` owns ordered response and block models in `db.messages.responses` and
`db.messages.blocks` for user, assistant, thinking, tool call/result,
authentication, and harness entries. The root managed view reprojects those
models. Stable `transcript/response-*` and `transcript/block-*` lifecycle events
append stream chunks without repeatedly copying accumulated responses, then
compact each block once at finalization. Thinking and active assistant blocks
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
quotes use the thinner `▏` rail. The focused `markdown`
extension performs a pure parse into semantic blocks and inlines; `component.markdown` turns that data into
terminal flow, while `component.message` supplies the outer rail, block surface, and collapsed previews.
`component.tool` composes compact lifecycle headings and reusable content views;
tool descriptions remain API documentation and are not displayed in the transcript.
`tool_presentations` binds tools to generic field, text, code, numbered-line, and
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
Languages still select the `syntax` extension's asynchronous captures when their grammar is installed.
Code renders plainly while highlighting is pending; stale streaming results are
discarded and the latest source is requested. Rendering consumes capture data
without loading grammars or calling a native parser. The pure `layout`
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

Load `component.content`, `component.truncation`, and `tool_presentations` before
`component.tool`; load `component.truncation` and `component.group` alongside `component.message`.
Presentation extensions can register a tool binding without modifying its definition:

```fennel
{:type :register/tool-presentation :name :query
 :value {:subject :database :fields [:database :sql] :code :sql :language :sql :numbered false}}
```

A binding may instead be a pure function returning
`{:arguments [{:role :content.fields :model {...}}] :result {:role :content.text :model {...}}}`.
The generic roles accept ordinary content: `content.fields` takes labeled values,
`content.lines` takes numbered rows with optional source offsets, and
`content.truncated` takes rendered lines and a visible-line limit. Applications
choose the bindings; renderers never inspect tool definitions or execute tools.
For streaming, `misa.markdown.new_document():update(text)` retains completed
blocks and reparses the final two blocks, where appended syntax can change the
interpretation. Replacements rebuild the document; unchanged normalized source
reuses it. The unfinished block can still reflow, and a large unfinished fence,
table, or paragraph is reparsed as a unit. Ordinary inline text is scanned in runs;
terminator searches retain their next match or failure so malformed openers do
not repeatedly search the same suffix. Quote prefixes and highlighted code lines
are scanned by source offset. Deep list/quote indentation is fitted to the
viewport without changing the parsed depth or hiding the body.
`misa.markdown_view.project(text, options, previous?)` returns an immutable
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
`agent` emits explicit stable response/block start, delta, end, and interruption
`transcript/*` events plus `agent/status` and `agent/usage`; visual extensions do
not inspect or reconstruct shadows from its private orchestration state. UI cancellation
dispatches the semantic `agent/cancel-active` action, and the agent alone resolves
that action to its active provider operation or every pending native tool call ID.
Cancellation intent remains set while completions are drained, prevents model
continuation, then returns the harness to ready with visible cancelled/interrupted
transcript state. Providers
normalize every response to `agent/stream-start`, `agent/stream-delta`, optional
`agent/stream-usage`, and `agent/stream-end`/`agent/stream-error`. Deltas cover
text, thinking, and incrementally assembled tool calls; finalized tool argument
chunks are compacted once before the transcript is reprojected. The focused `json`
extension supplies `misa.json.decode` and `misa.json.encode` at protocol
boundaries. Before continuation, the agent parses every tool call into a
provider-neutral `arguments` table and removes transport JSON; adapters encode
that table only when their wire protocol requires a JSON string. This also
keeps unknown-tool calls replayable after a provider switch. Only the agent correlates
request IDs and promotes a completed assistant response into provider history;
an interrupted response remains marked in the visible transcript but is not
replayed to the provider.

Agent delta assembly is extensible through
`{type="register/agent-delta", id="my_delta", value=function(stream, delta, request_id) ... end}`.
Pure handlers return `{patch=<stream-state patch>, fx=<ordered array>}` or nil.
The agent applies request/cancellation correlation before dispatch. Handlers
assemble canonical text, thinking, or tool-call blocks and emit transcript
events; they do not mutate prior stream state or append conversation history.

Transcript block updates expose
`{type="register/transcript-delta", id="block_kind", value=function(block, event, policy) ... end}`.
Pure reducers return a block patch or nil; the transcript owner applies it without
mutating the event or earlier blocks. `policy` provides the configured preview
limits and redaction keys. This registry handles deltas, not block creation.

The transcript owner publishes `{type="transcript/updated", response_id=...,
block_id=...}` after changing blocks. `block_id` is optional for a whole-response
update. Consumers call `misa.transcript_blocks(db, response_id, block_id?)` for
the current immutable blocks without depending on response indices or storage
layout. Missing targets return an empty array. An explicit notification without
`response_id` requests processing of all blocks, for bulk imports that install
canonical transcript state. Notifications identify what to read, not a copied
text snapshot; queued duplicates safely observe the latest committed source.

`register/transcript-presentation` takes `id=<block kind>` and
`value=function(model, transcript_state, selected_range) ... end`. Pure projectors
return `{role=<component role>, model=<additional render fields>}` or nil to omit
the block. Selection is nil for other blocks. These projections choose rendering
without changing transcript facts or the component's returned line collection.

`indicators` is a focused registry for semantic status values. Features return
`{type="register/indicator", value={id,label?,icon?,hotkey?,query={"query-id",...}}}`
in their setup effects. Queries use the existing subscription graph and explicit
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

`values` installs an open, pure value-rendering dispatcher, independently of
status or transcript components. Load it before those consumers. Extensions
add `{type="register/value-renderer",id="my-type",render=function(fact,context)
... end}` returning semantic spans. `misa.render_value(fact,context?)` invokes it;
unknown types and duplicate registrations fail explicitly. The default component
uses shared compact-number, percentage, and currency formatters, while owning
styles and width dropping. Response cost metadata is a money fact too, including
`pending=true` with no amount before completion. Timestamps reach components as
raw `started_wall_ms`; the `timestamp` value renderer formats milliseconds as
UTC time-of-day. The old formatted response `text` and `cost_format` service are removed.
Animation selection is separate presentation data from
`misa.animation_presentation(db,role)` / `[:animations/presentation role]`; the
activity renderer turns it into clock-driven spans without changing activity facts.

`request_options` derives request readiness and selected option values entirely
from the active model's `api.request_options` metadata. Required options without
values block a request before the user turn is recorded and emit a structured
harness problem. `effort` is the reasoning-effort affordance: `/effort` lists
only the selected model's declared choices, and the configurable global
`cycle_effort` binding (default `alt+f`) cycles those choices. Model switches
retain an equivalent value when supported, otherwise use the new model default;
models without reasoning support simply expose no effort value.

`choices` owns generic choice state and narrowing transitions shared by inline
completion and overlays. The item contract separates stable `id`, emitted
`value`, semantic `display`, hidden `search`, optional `preview`, and `path`.
Choice sources are registered with `{type="register/choice-source", id=id, value=source}`; any item can narrow
to another source, and empty-query Backspace pops the whole narrowing frame.
Selected rows retain the standard selected style in every panel; only the
active leftmost panel receives the `>` focus marker.

Choice views are immutable implementations registered with
`{type="register/choice-view", id=id, value=view}`. The built-ins are `all`, `favorites`, `frecency`, and
`browse`. Browse shows up to three recent choices above the remaining choices;
searching produces a single ranked list. `config.choices.recent_limit` adjusts
that count. Model and argument pickers use Browse beside Favorites by default.

Choice sessions are immutable data. `choice_refresh`, `choice_set_items`, and
`choice_replace_view` return a new session; callers must retain that return value.
`choice_input` and `choice_accept` return a result containing `session` alongside
outcomes such as `accepted`, `cancelled`, and `narrowed`. Rows and layout never
mutate the supplied session. Add input transitions with
`{type="register/choice-input", id="my_action", value=function(session, event, db) ... end}`;
the handler returns the same result shape and must leave its inputs unchanged.

Editor handlers also return immutable patches. Register an additional text-edit
kind with `{type="register/editor-edit", id="my_edit", value=function(editor, event) ... end}`.
It returns the next editor data, without mutating either input; slash-choice
synchronization then runs over that result. Submission and modal choice outcomes
remain separate from these text edits.

The Vim policy accepts `register/editing-motion` and `register/editing-action`
effects with `id` and `value`. A motion receives editor data and returns a
grapheme-boundary byte offset. An action receives `(editor, editing, event, db)`
and returns `{editor=<patch>, editing=<patch>, fx=<array>}`. Both are pure;
the policy handles undo bookkeeping and selection projection around the result.
Editor-owned transitions call the optional pure
`editing_transition(editing, previous_editor, next_editor, reason, interactive)`
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

Inline and overlay sessions resolve the same `keybindings.choices` actions and
positional banks. `choice_layout` is the single projection for responsive
preferred/min/max overlay bounds, preview and panel allocation, shared hints,
and positional targets. The picker component only renders that projection.
Load `keybindings` before choice layout and the editor, picker, and status
components. All key hints use its shared token renderer, preserving input case;
equivalent encodings such as `ctrl_n` and `ctrl+n` produce identical spans.
Load `choice_preview` before `choice_layout`. It renders typed preview data to
semantic lines before geometry is calculated; compact and overlay input handling
use the same resulting line heights and targets. Extensions register
`{type="register/choice-preview",id="my-preview",render=function(model,context)
... end}`, returning semantic lines. `context` supplies `columns` and `compact`.
Use `config.choices.preview_renderers` to map a preview type to a renderer ID;
unknown IDs and duplicate registrations fail explicitly.

Model previews carry `type="model"`, raw `context_window`, and `cost` facts from
`misa.model_cost_info`: `currency`, `token_unit`, `pricing`, `estimated`, and
`unavailable`. Rate numbers are not converted to strings by model/cost owners.
The default preview renderer owns summary wording, currency precision, and layout.
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

`dialogs` owns correlated modal/progress/alert lifecycle, actions, cancellation,
and optional text input. Buttons activate by click or their configurable binding;
there is no implicit first action. Only an action with `primary=true` responds to
Enter. A button may declare `confirm={title,message,label}` to require the shared
confirmation flow, and `persistent=true` to keep the original dialog open after
completion. Escape cancels a confirmation first, then dismisses the dialog.

Dialogs may carry `sections=[{id?,title?,heading?,rows=[{label,fact,meter?,detail?,actions?}]}]`.
Facts and details use `values`' typed formatting; optional meters carry `used` and
`limit`. Row action IDs refer to the dialog's action definitions, with `inline=true`
placing their buttons in the row. `component.data` measures shared columns across
all sections, and `component.buttons` uses the ordinary keybinding and action span
presentation. `dialog_view` composes data with the replaceable dialog chrome.
`dialog/update` replaces supplied sections as a collection while preserving open
confirmations and scroll position. Arrow, wheel, and page keys scroll overflow.
Usage supplies this semantic data contract and owns no rendering or input model.
Dialog handlers return patches. Additional ordinary input kinds use
`{type="register/dialog-input", id="my_input", value=function(dialog, event) ... end}`;
the pure handler returns `{patch=<application patch>, fx=<array>}`. Protected
dialogs bypass ordinary input handlers entirely.
A protected dialog starts `input/protected` for a waiting native operation and
correlation. Its bounded 64 KiB buffer stays native; Lua receives only length,
submission/cancellation, and capacity-error metadata. Buffered input is held
until capture is installed and scrubbed if the operation fails or is cancelled,
so a pasted key cannot fall through into a conversation.
`dialog_view` projects that state through the replaceable
`dialog` component role. Dialog data and hints are generic—providers do not own
UI paths or rendering. Default dialogs are compact overlays that retain the
transcript, with clickable URLs and wrapped input. `picker` is only the overlay lifecycle adapter and
`picker_view` renders the centralized projection. `commands` normalizes every
typed, picked, or replayed command into one canonical invocation and records
recent invocations in the generic `commands` preference scope. Canonical strings
(such as `/model vendor/model`) are also favorite IDs. Usage is recorded once
per invocation, including typed commands; replay and registered commands share
the same identity. `omnipicker` (global `alt+/`, also used by the
slash menu) composes commands with those recents; commands with completion
narrow to argument choices before emitting that same canonical event.
`models` owns only model catalogue, availability, rich metadata preview, and
selection policy; `/model` has no model-specific picker behavior.
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
transcript, and usage. `agent` owns normalized conversation history,
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
`tool.files` registers `read_file`, `list_directory`, `write_file`, and
`edit_file`; `tool.shell` registers `shell`. They are ordinary explicit Fennel
extensions and are not enabled by the harness. `misa mcp` exposes the same
Extension-registered schemas and effect translators as an MCP stdio server. The
Claude provider supplies this bridge through `--mcp-config` whenever tools are
registered, while retaining `--tools ""` so Claude's own tools remain disabled.
The MCP child inherits `MISA_CONFIG`; configurations selected with `--config`
should set `config.providers.claude.mcp_arguments` to
`["mcp", "--config", "/the/same/config.json"]`. `mcp_command` defaults to
`misa` and may be set to an absolute executable path.
`provider.fake` keeps its state under
`db.providers.fake`; `provider.command` adapts user executables.
`provider.claude` invokes Claude Code's stream-JSON process protocol and reuses
Claude's existing Pro/Max credentials without copying them into Misa. Its
catalogue uses the current full model IDs from Anthropic's model documentation:
Fable 5.1, Opus 5, Sonnet 5, and Haiku 4.5. At startup Misa reads
`subscriptionType` from `claude auth status`; Max accounts get a 1M Opus
context window and other plans get 200k. `config.providers.claude.max_plan` is
an optional boolean override for environments where status discovery is
unavailable. API-backed Anthropic catalogues are
refreshed from `GET /v1/models` for the models available to that API key.
`editor` owns multiline UTF-8 editor state and transitions. `messages` owns
transcript scrolling and bounded window extraction. `models`, `request_options`,
and `messages` expose narrow read-only projections used by status and composition;
those consumers never traverse feature-private state. `ui` owns only root
composition. Interactive sessions return to the editor after each response;
explicit argv remains a single headless turn.

`costs` exposes model rates, response totals, and a session status indicator.
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
The `usage` extension projects provider facts into the generic `data` component.
Its rows share label and meter columns, with typed values formatted by `values`.
Dates use the local locale/time zone and relative durations, updated once a minute.
Dialog chrome, scrolling, click/hotkey buttons, and confirmations are shared UI
primitives. Usage has no primary action: Enter does nothing until a confirmation
is open. Escape dismisses confirmation first, then closes Usage and resumes input.
Concurrent refreshes coalesce, stale completions are ignored, and failures clear
stale quota values.

Codex reset details come from `/wham/rate-limit-reset-credits`; the dashboard shows
the available count and each returned expiry. The inline **Use reset** button (`r`,
configurable through `keybindings.usage.codex-reset`) requires confirmation before
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
monthly limit. **Manage** (`e`, configurable through `keybindings.usage.extra-manage`)
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

Assign models with `/role default provider/model` and
`/role summarizer provider/model`; `/role summarizer off` removes the latter.
Assignments persist and can be overridden by `config.models.roles`.
Extensions resolve full available models through `misa.model_for_role(db, role)`;
an unassigned or unavailable background role resolves to nil.

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
`"plain"` to disable modal editing, or replace the `editing` extension. Bindings
are configurable under `config.keybindings["editor.normal"]`.

`history` records accepted submissions, deduplicating consecutive identical
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
`queue` owns scheduling, `queue_view` owns its dock, and the editor targets the
installed submission capability. These plugins can be replaced independently.
The dock renders the `pending-prompt` component role with
`{pending=<raw text>, attachment_count=<number>}`. Draft attachment controls use
`attachment-controls` with `{pending=<boolean>, count=<number>}`. Their default
components keep existing resting styles and use shared action-hover backgrounds;
override either role through `config.components.roles` to change presentation.

Extensions contribute lifecycle facts through named subscription queries, not
by rewriting completion or input events. Register a subscription and expose its
query with `{type="register/service", name="editor_lifecycle.<extension>",
value={"<query-id>"}}`. Registration may precede or follow the editor extension;
duplicate names are rejected by the service registry. Each query declares its
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
`images` owns acquisition policy, `attachments` composes the draft, and the
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

`clipboard` separates copying from selection. Its default `clipboard/write`
native effect sends OSC 52 to an interactive terminal (up to 1 MiB); terminal
clipboard support must be enabled. Configure `config.clipboard.command` with
an argv array such as `["wl-copy"]`, `["xclip", "-selection", "clipboard"]`, or
`["pbcopy"]` to send copied text to that process's stdin instead. The internal
register always remains available for editor paste.

Features return `{type="register/action", value={id,label,event,keys?,binding?,available?}}`
in setup effects; `binding` identifies a configured `{context,action}`, and `available(db)`
controls contextual discovery. Without `binding`, an action gets a global
binding under its ID; `keys` supplies optional defaults. Configure it through
`config.keybindings.global[id]`. The `actions` extension routes global action
bindings and supplies the palette.
`selection` accepts `{type="register/selection-source", id=id, value=function(db) ... end}`
in setup effects, with the function returning
source documents with `{id,label,text,kind,first,last,children}` and unique string
IDs across all sources. Ranges are
zero-based, half-open byte offsets. `selection_document` derives semantic
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
and documents; `v` again returns to the focused node. `selection_ranges(db)`
returns ordered `{id,text,kind,first,last}` slices of the frozen sources.
`selection_projection(db, id?)` returns the selected slice for an ID, or the
focused document when no ID is supplied. Copy preserves each slice's original
bytes and inserts a blank line (`\n\n`) between documents. The viewport follows
the focused document, not the beginning of the whole range.
Selection actions are extensible through
`{type="register/selection-action", id="my_action", value=function(state, db, event) ... end}`.
Pure handlers return `{state=<new selection state>, fx=<array>, close=<boolean>}`.
Omitted state is unchanged; `close=true` dismisses selection. Copying is one
action, not a requirement of navigation or range projection.

## Nix

NixOS, nix-darwin, and home-manager expose the same
`programs.misa.extensions` option. Nix path values select custom extensions;
bare strings must be catalog IDs. Generated config is world-readable in the
Nix store, so it must not contain secrets.

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
symbol.

`default.nix` exports the package, overlay, shell, modules, `lib`, and
`standardExtensions`. Raw and Nix configurations use the same ordered list:

```nix
let p = import ./path/to/misa { inherit pkgs; }; in
p.lib.mkMisa {
  extensions = with p.lib.standardExtensions; [
    values json providerFake keybindings links actions
    themes themeDefault animations animationDefault components layout choiceLayout markdown componentMarkdown indicators dialogs componentButtons componentData dialogView componentTool componentMessage componentEditor componentPicker componentStatus componentChrome componentDialog
    messages status usage picker pickerView models omnipicker requestOptions effort agent editor ui
  ];
  config = {
    models.default = "fake/default";
    providers.fake.responses = [ "done\n" ];
  };
}
```

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
and `misa logout PROVIDER` to remove it. The `auth` extension provides the same
flows inside the TUI as `/login PROVIDER`, `/status PROVIDER`, and
`/logout PROVIDER`. For `claude`, all three commands are
delegated to `claude auth`. Interactive API-key login uses the ordinary popup
with masked native input. CLI login still hands the terminal to Claude and
returns automatically; logout does not need a handoff. OAuth access credentials
are refreshed from their
stored refresh tokens. Credential mutex acquisition is cancellable and refresh
network I/O never holds a mutex. A refreshed token is published with a
process/interprocess-locked compare-and-swap over credential generation, refresh
token, and profile; logout or a newer login always wins and stale refreshes can
never resurrect it. Mutations take a sibling interprocess lock, reload the latest
document, merge one provider, and atomically replace it, so concurrent Misa
processes do not lose unrelated grants. Kimi defaults to the official global `.ai` profile; set
`config.providers.kimi.region` to `"mainland"` for the `.com` endpoint bundle.
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

API provider lists include their protocol explicitly, for example
`[ "json", "protocol.anthropic", "provider.anthropic", "components", "layout", "markdown", "component.markdown", "component.tool", "component.message", "component.editor", "component.picker", "component.status", "component.chrome",
"messages", "models", "request_options", "effort", "agent", "commands",  "choices", "choice_layout", "editor", "ui" ]` (add `picker`, `picker_view`, and `omnipicker` when overlay choices are needed).
OpenAI and OpenRouter use `protocol.openai`; Anthropic and Kimi use
`protocol.anthropic`. ChatGPT subscription access is the separate
`provider.openai-codex` extension and its Codex Responses protocol.
OpenAI-compatible delta projections can be extended with
`{type="register/openai-delta", id="my_delta", value=function(delta, record) ... end}`.
Pure projections return arrays of normalized agent deltas and run in registration
order, after the built-in text, reasoning, and tool-call projections.
Anthropic-compatible streams expose `register/anthropic-record`,
`register/anthropic-block-start`, and `register/anthropic-block-delta` with `id`
and `value` fields. Pure handlers receive `(state, record, request_id, provider)`
and return `{patch=<stream-state patch>, fx=<array>, finish=<boolean>}`. This keeps
signed provider state separate from visible thinking deltas. Terminal handlers
set `terminal=true` or `failed=true` and request `finish=true`.
Codex record handlers are pure and extensible through
`{type="register/codex-record", id="record.type", value=function(state, record, request_id) ... end}`.
Handlers return `{patch=<stream-state patch>, fx=<array>, finish=<boolean>}`.
Completion records mark `terminal=true`; failures mark `failed=true`. Either
stops subsequent records from emitting output for that stream.

Claude CLI records expose `register/claude-record` and
`register/claude-stream-event`, with `id` and `value` fields. Pure handlers receive
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
Shell previews retain that output tail even when a background summary exists.
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

The optional `tool_summary` extension (included in the default profile) uses the
`summarizer` model role for tool-free background summaries of successful tool
results longer than 240 bytes. Requests run one at a time; short results and
errors keep their direct previews. Summaries affect collapsed presentation only;
canonical tool results remain unchanged, and provider failures retain the direct
preview. Summary usage is included in both its originating response group's cost
and the session total, while provider throughput remains independent. Resetting the conversation
cancels pending summaries. No summary requests run until a model is assigned to
the role, for example `config.models.roles.summarizer = "provider/model"`.
Turning the role off or losing model availability cancels active summaries and
clears the queue; changing the role model restarts unfinished work on the new
model. Late completions from cancelled requests cannot change the transcript.
