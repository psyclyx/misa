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
    "component.dialog",
    "component.selection",
    "dialog_view",
    "messages",
    "status",
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
Run standalone policy tests with `tools/fennel tests/runtime-state.fnl`. The same
runner supports the Fennel benchmark scripts and uses the bundled compiler.

Optional
terminal-protocol regressions run with `python3 tests/ghostty-input.py
zig-out/bin/misa`, `python3 tests/settled-frames.py zig-out/bin/misa`, and
`python3 tests/threaded-terminal.py zig-out/bin/misa`. They
use a PTY and local provider fixtures, with no account or network dependency.

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
- `{type="register/interceptor", value={id=..., before=fn?, after=fn?}}`: before callbacks run
  in registration order and after callbacks in reverse. They receive and may
  return `{db,event,cofx,fx}`.
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
- `{type="register/sub", value={id=..., inputs=fn, compute=fn}}` (or `value={id=..., read=fn}`): registers a pure
  query projection. Call it as `misa.sub(db, [id, ...args])`; `inputs` returns query vectors
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
state. The `syntax/projection` subscription depends on one document entry and
passes that data into rendering; retained states do not depend on an external
cache. Projection results are immutable, including when adding layout options.
Source parsing and grammar
loading run on native workers with a reusable parser cache. Missing, unknown,
incompatible, or timed-out grammars produce plain-text fallback. Source is
limited to 1 MiB. There is no synchronous highlighting API on `misa`.
Recursive dispatch is
unavailable. State commits retain unchanged branches by identity, so subscriptions can cheaply
reuse projections that do not depend on the changed paths. Handlers, coeffect derivations,
interceptors, and projections receive persistent state directly: dispatch does not clone or
reconcile the database. Interceptors update state with `misa.patch`, not nested mutation.
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
the session transfers the semantic view arena to the terminal owner. The working
`db` copy and projection/input snapshots still enforce their existing rollback
and mutation-isolation contracts.

HTTP deadlines describe only signals the transport can observe: `first_byte_ms`
includes DNS and connection establishment, `idle_ms` applies after response-body
activity begins, and `overall_ms` bounds the complete operation. There is no
separately configurable connect deadline.

The fixed native effects are:

- `{type="dispatch", event=<table>}`
- `{type="syntax/highlight", language=..., source=..., completion=..., id=..., timeout_ms=?}`
- `{type="process/run", argv={<strings>}, completion=<event type>, id=<string>, timeouts={startup_ms=?,idle_ms=?,overall_ms=?}}`
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
ID inside Zig, so secret bytes never cross into Fennel policy. `process/run` invokes direct argv,
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
 :cursor {:row 1 :byte 0}} ; or nil cursor
```

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
synchronization logic. Unsupported terminals retain the buffered-write fallback.
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
`component.message`, `component.tool`, `component.markdown`, `component.editor`, `component.picker`, `component.status`,
`component.chrome`, and `component.dialog` plugins provide the default roles; none owns behavior or
depends on a theme. `layout` provides pure terminal-cell width, fitting,
semantic-span wrapping, and responsive-column primitives. Theme resolution is
centralized at the component registry boundary. Custom code calls
`misa.render_component(db, role, model, context)` and
`misa.animation_span(db, role, options?)` for clock-driven visual motion.
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
responsive semantic indicator row remain visible below it. Default message roles
and timestamps occupy a title line above the heavier `┃` message rail instead of prefixing
content. Markdown quotes use the thinner `▏` rail. Completed assistant responses show tok/s exactly once, on their final assistant
block, and only from provider-reported output tokens and positive native monotonic elapsed time. The focused `markdown`
extension performs a pure parse into semantic blocks and inlines; `component.markdown` turns that data into
terminal flow, while `component.message` supplies only message titles and the outer message rail. The reusable `component.tool`
owns tool name/description, arguments, pending state, result, and lifecycle colors. Rendering includes visibly graded
streaming headings, composable emphasis, Unicode task checkboxes, nested lists and continuations, thematic rules,
quotes, responsive bordered tables, inline/fenced code, and OSC 8-capable links. Fenced languages are labeled and use
the `syntax` extension's asynchronous captures when their grammar is installed.
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
cutoff, and long messages retain their formatting. Assistant and user text is
retained in full, including streaming chunks. `config.messages` controls initial `verbose`
and the tool-preview `redact_keys`, `max_string`, `max_items`, and `max_depth`
limits. Headless output remains plain.
For streaming, `misa.markdown.new_document():update(text)` retains completed
blocks and reparses the final two blocks, where appended syntax can change the
interpretation. Replacements rebuild the document; unchanged normalized source
reuses it. The unfinished block can still reflow, and a large unfinished fence,
table, or paragraph is reparsed as a unit. Ordinary inline text is scanned in runs;
terminator searches retain their next match or failure so malformed openers do
not repeatedly search the same suffix. Quote prefixes and highlighted code lines
are scanned by source offset. Deep list/quote indentation is fitted to the
viewport without changing the parsed depth or hiding the body.
`misa.markdown_view.new_document():render(text, options)` adds per-block layout
reuse keyed by width and semantic styles. Both APIs return read-only derived
snapshots. Default message components keep these documents for the transcript's
lifetime and copy cached bodies before applying titles, rails, and theme colors.
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

`register/transcript-presentation` takes `id=<block kind>` and
`value=function(model, transcript_state, selected_range) ... end`. Pure projectors
return `{role=<component role>, model=<additional render fields>}` or nil to omit
the block. Selection is nil for other blocks. These projections choose rendering
without changing transcript facts or the component's returned line collection.

`indicators` is a focused registry for semantic status values. Features return
`{type="register/indicator", value={id,label?,icon?,hotkey?,value=function(db)...end}}`
in their setup effects.
`config.status.indicators` selects order, label/icon representation, an optional
structured keybinding reminder, and drop priority. The common component styles the standard
`label`, `value`, and `keybinding` tokens independently and
removes low-priority items at narrow widths. Standard registrations cover
activity, model, effort (with its cycle hotkey), session usage, context usage,
and transcript detail (`summary`/`verbose`). Root composition stays generic.

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
Overlays remain bounded, nonexclusive regions of the managed root: query input
comes first, then semantic preview, panels, and the shared key reference below
the panels. All key hints use structured tokens and render Alt as `⌥`. `open_overlay` (default `alt+space`) promotes the current inline
session without resetting its query, highlight, or narrowing context. `replace_view`
(default `alt+/`) opens the nested `picker-picker`; replacing the active view is
kept only for that choice session and never changes purpose defaults.

`dialogs` owns correlated modal/progress/alert lifecycle, actions, cancellation,
and optional text input. Tab or Left/Right selects among multiple actions.
Dialogs may carry `sections=[{id?,title?,fields=[{label,value}]}]` alongside ordinary
message text. Nil-valued fields are omitted; numbers and booleans remain typed
until presentation. `dialog/update` replaces supplied sections as a collection.
The usage dashboard uses this data contract, including deterministic provider
ordering, rather than inserting a pre-rendered report string.
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
Provider-reported USD takes precedence over estimates; missing prices remain
explicitly unknown. Dynamic provider metadata supplies prices where available.
`config.costs.models["provider/model-id"]` overrides `input`, `output`,
`cache_read`, and `cache_write` rates in USD per million tokens, plus optional
`request` in USD per request. A response snapshots its model rates when it
starts. Selectors always display canonical `provider/model-id`; friendly model
names remain searchable. Enter and Space after `/model` enter the same inline
argument choices, and accepting a model executes the command.

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

Alt-S enters structural transcript selection. `j`/`k` select siblings, `l`
narrows, and `h` widens. A message can narrow to sections, a section's heading
or content, paragraphs, code blocks, tables, rows, cells, lines, words, and
individual graphemes. The selected range is highlighted in the existing rich
transcript; a small input dock shows its path and keys. `y` copies its exact
source, including Markdown syntax; Escape returns to the editor. The selected
source is frozen so streaming cannot move the range before copying. F1 remains
available. The `selection` component role renders the dock independently of
transcript decoration. Page Up/Down, Alt-K/J, and the mouse wheel scroll the
transcript; its viewport stays anchored while new content arrives.

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
source documents with `{id,label,text,kind,first,last,children}`. Ranges are
zero-based, half-open byte offsets. `selection_document` derives semantic
ranges from Markdown and lazily supplies finer ranges; selection policy and
rendering can both be replaced independently.

Within a document, `v` anchors a visual range and sibling motions extend it;
`v` again returns to the focused node. Changing structural depth or documents
resets the range. Selection actions are extensible through
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
    themes themeDefault animations animationDefault components layout choiceLayout markdown componentMarkdown indicators dialogs dialogView componentTool componentMessage componentEditor componentPicker componentStatus componentChrome componentDialog
    messages status picker pickerView models omnipicker requestOptions effort agent editor ui
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
