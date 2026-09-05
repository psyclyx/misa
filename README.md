# misa

misa is a small event-driven coding-agent harness built with Zig 0.16,
system LuaJIT, and system tree-sitter. The TUI uses only the Zig standard
library.

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
    "choices",
    "preferences",
    "themes",
    "theme.default",
    "animations",
    "animation.default",
    "components",
    "layout",
    "indicators",
    "component.message",
    "component.editor",
    "component.picker",
    "component.status",
    "component.chrome",
    "messages",
    "status",
    "picker",
    "picker_view",
    "models",
    "request_options",
    "effort",
    "agent",
    "editor",
    "ui"
  ],
  "config": {
    "models": { "default": "claude/claude-sonnet-5" }
  }
}
```

```sh
zig build
zig build test
zig build test-nix
zig build -Doptimize=ReleaseSafe
zig build run -- --config config/default.json hello
```

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
`.lua` are literal custom paths.

## Event, coeffect, effect, and view contract

An extension returns `{ setup = function(context) ... end }`. There is no `run`
phase. Setup may register:

- `misa.reg_event(type, handler)`: handlers run in registration order and thread
  canonical Lua `db`. A handler receives `(db, event, cofx)` and returns nil or
  `{db=<table>, fx=<ordered array>}`.
- `misa.reg_interceptor({id=..., before=fn?, after=fn?})`: before callbacks run
  in registration order and after callbacks in reverse. They receive and may
  return `{db,event,cofx,fx}`.
- `misa.reg_cofx(name, fn)`: derives a policy value. Derivations run in
  registration order; a later derivation may read values installed by earlier
  ones. Base coeffects always include `config`, `argv`,
  `terminal={interactive=<bool>,columns=<integer>,lines=<integer>}`, and
  `clock={wall_ms=<Unix epoch milliseconds>,monotonic_ms=<monotonic milliseconds>}`.
  These names are reserved. Native code samples both clocks for every
  transaction; elapsed durations must use `monotonic_ms`, never wall time.
  Configuration and argv are shared immutable inputs by contract.
- `misa.reg_fx(type, fn)`: translates a Lua policy effect to one native effect
  or an ordered array of native effects.
- `misa.reg_view(fn)`: registers exactly one semantic projection.
- `misa.reg_view_layer(id,fn)`: contributes an optional semantic overlay layer;
  the UI composes layers without knowing plugin-owned state.
- `misa.reg_model(model)`: adds a provider-owned catalogue entry, including an
  optional `context_window` and API metadata. `api.request_options` maps generic
  option names to `{choices={...},default=...,required=?}` declarations and
  `api.request_options_serializer` identifies the provider transport serializer.
- `misa.reg_auth_provider(provider)`: declares an authentication ID, model
  provider, native `strategy`, and (for OAuth) trusted endpoint `profile` for
  completion and availability tracking. Native validation rejects drift.
- `misa.reg_command({name,description,event,completion?,complete?})`: adds a
  generic slash command. `completion` names a shared static candidate group;
  `complete(prefix,db)` supplies dynamic candidates when needed.
- `misa.reg_completion(group,{value,label?,description?})`: lets any plugin add
  a candidate to a shared completion group. The UI handles filtering, sorting,
  display, and insertion, so providers only declare their authentication ID.
- `misa.reg_tool(tool)`: adds a semantic tool schema and its effect type.
  `misa.models()`, `misa.model(id)`, `misa.auth_providers()`, `misa.commands()`, `misa.command(name)`,
  `misa.tools()`, and `misa.tool(name)` expose the sealed registries.

Framework registrations are sealed after setup; component, theme, and animation
registries additionally seal as `app/start` begins. The checked synchronous
`misa.syntax.highlight(language, source)` native API returns ordered
`{start_byte=<zero-based>, end_byte=<exclusive>, capture=<semantic name>}`
ranges. Captures use a finite generic vocabulary (`comment`, `string`, `number`,
`keyword`, `type`, `function`, `constant`, `variable`, `property`, `tag`,
`attribute`, `operator`, `punctuation`, `escape`, and `embedded`). Highlighting
is derived only and never enters canonical `db`. Missing, unknown, or
incompatible grammars return an empty array for plain-text fallback. Source is
limited to 1 MiB and parsers are loaded lazily into a bounded cache.
Recursive dispatch is
unavailable. Each transaction takes one bounded working copy of `db`, then
commits it only after its effects and view pass native validation and
presentation. Projections receive private snapshots, so projection mutation can
never change transactional state. Extensions must
not write or render: `io`, `os`, and `print` are unavailable. Ordinary Lua
source composition (`require`, `load`, `loadfile`, and `dofile`) remains
available, while native `package.loadlib`, FFI, and JIT access are disabled.

HTTP deadlines describe only signals the transport can observe: `first_byte_ms`
includes DNS and connection establishment, `idle_ms` applies after response-body
activity begins, and `overall_ms` bounds the complete operation. There is no
separately configurable connect deadline.

The fixed native effects are:

- `{type="dispatch", event=<table>}`
- `{type="process/run", argv={<strings>}, completion=<event type>, id=<string>, timeouts={startup_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="http/request", url=..., json=..., credential=..., completion=..., id=..., timeouts={first_byte_ms=?,idle_ms=?,overall_ms=?}}`
- `{type="file/read", path=..., completion=..., id=...}`
- `{type="file/list", path=..., completion=..., id=...}`
- `{type="file/write", path=..., content=..., completion=..., id=...}`
- `{type="file/edit", path=..., content=..., replacement=..., completion=..., id=...}`
- `{type="json/decode", source=..., completion=..., id=...}`
- `{type="terminal/read"}`
- `{type="timer/start", interval_ms=<10..60000>, completion=..., id=...}` / `{type="timer/stop", id=...}`
- `{type="operation/cancel", id=...}`
- `{type="auth/command", action=..., provider=..., strategy=..., profile=...,
  completion=..., interaction=..., id=...}` / `{type="auth/respond", id=...,
  correlation=..., action=..., value=...}`
- `{type="state/load", namespace=..., completion=...}` / `{type="state/save", namespace=..., data=...}`
- `{type="view/commit", lines=<semantic lines>}`
- `{type="app/quit"}`

Unknown native effects fail the session. `http/request` injects credentials by
ID inside Zig, so secret bytes never cross into Lua policy. `process/run` invokes direct argv,
never a shell, captures stdout and stderr with 1 MiB bounds, and never inherits
the terminal output. The shell tool explicitly translates its command to
`{"sh", "-lc", command}` in Lua; filesystem and process isolation are concerns
of the environment launching Misa, not of the tool extension. File effects use
paths exactly as supplied and bound file content to 1 MiB. Captured tabs are
normalized to spaces and malformed UTF-8
is repaired before completion events are dispatched. Completion events include
`ok`, `status`, `stdout`, `stderr`, and `id`. With
`stdout_format="json_lines"`, successful output is decoded into an ordered
`records` array instead of being returned as an opaque string. `process/run`
also accepts bounded `stdin` text or one `stdin_json` value; the latter is
serialized with a trailing newline for JSONL subprocess protocols.

A view is modest semantic data:

```lua
{
  lines = {
    { spans = { { text = "working", style = "dim" } } }
  },
  cursor = { row = 1, byte = 0 } -- or nil
}
```

Native styles are `plain`, `dim`, `bold`, `accent`, `user`, `assistant`, and
`error`; themes also map component tokens such as `indicator.label`,
`indicator.value`, and `indicator.hotkey` onto those native styles.
Text must be valid UTF-8 and may not contain controls, ESC, CR, or LF. Cursor
rows are one-based; `byte` is a zero-based UTF-8 boundary in the concatenated
spans of that row. Zig alone converts it to a terminal cell column and clamps it
to the presentable width. Zig owns clipping and ANSI mapping; noninteractive
output strips styles.

## Terminal architecture and limitations

`src/session/root.zig` owns a non-reentrant FIFO event loop and parses effects
once into a closed native union; `src/capability/process.zig` owns direct
process execution and captured-output normalization, while
`src/capability/file.zig` owns bounded file operations. Lua owns canonical application
state. A transaction is fully validated before Lua policy state is committed. Views
are deadline-coalesced and the latest prepared frame is retained during terminal
handoff. Potentially blocking HTTP, process, authentication, file, credential,
and persistent-state work runs in cancellable native workers and reports
completion errors as events; the owner thread performs only Lua/rendering and
bounded queue work. Worker publication uses a bounded per-operation queue and a
nonblocking wakeup pipe. One readiness wait covers that pipe, requested stdin,
and the nearest timer, frame, operation-timeout, or decoder deadline; there is
no periodic operation polling. `src/terminal/root.zig`
owns tty/raw-mode lifetime, exclusive generation-checked handoff, and all stdout writes; `terminal/input.zig`,
`terminal/presenter.zig`, and `terminal/width.zig` own their respective pure
mechanisms. Raw mode is always restored with `defer`.

The inline presenter never enters the alternate screen and never clears the
screen or scrollback. Immutable commits use real newlines and are forgotten.
Only the mutable frame at the bottom is erased/repainted with CR, relative
movement, and per-row erase-line sequences; final cleanup erases only those
owned rows. Live logical lines are conservatively clipped to one physical row
before the final terminal column, while immutable commits may wrap naturally.
The installed default sets `config.ui.plain_prompt = true`, causing the standard
UI to commit one plain startup prompt before reading stdin when inline mode is
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
test dependencies. There is no PTY integration test: Zig's standard library
does not provide a portable POSIX PTY constructor, and misa does not add an
OS-specific helper or external dependency for one. Zig 0.16 no longer
exports `std.posix.read/write/isatty`; misa uses the corresponding
`std.Io.File` portable abstractions. Raw mode uses portable termios with a read
timeout, and POSIX signal actions restore it before chaining/defaulting normal
termination signals. Bracketed-paste markers are decoded, but misa does not
enable that mode because Zig exposes no portable async-signal-safe POSIX write.

## Standard extensions

The application UI is split at semantic boundaries. `components` resolves
visual roles to registered implementations. The independently loadable
`component.message`, `component.editor`, `component.picker`, `component.status`,
`component.chrome`, and `component.dialog` plugins provide the default roles; none owns behavior or
depends on a theme. `layout` provides pure terminal-cell width, fitting,
semantic-span wrapping, and responsive-column primitives. Theme resolution is
centralized at the component registry boundary. Custom code calls
`misa.render_component(db, role, model, context)` and
`misa.animation_frame(db, role, tick?)`. Configure individual roles with
`config.components.roles` (for example, `"picker": "my.picker"`) and dispatch
`components/swap` with `role` and `implementation` to swap one at runtime.
`themes`/`theme.default` and `animations`/`animation.default` are independent
data registries switched with `themes/swap` and `animations/swap`. Animation
roles are selected with `config.animations.roles`; the service advances their
transactional ticks using ordinary `timer/start` and `timer/stop` effects.
Selections live in `db`, so failed transactions roll back; successful swaps persist through
the generic state service. Set `persist = false` in the corresponding config
section to disable persistence. These registries contain no input or agent behavior.

`messages` owns ordered response and block models in `db.messages.responses` and
`db.messages.blocks` for user, assistant, thinking, tool call/result,
authentication, and harness entries. The root managed view reprojects those
models. Stable `transcript/response-*` and `transcript/block-*` lifecycle events
append stream chunks without repeatedly copying accumulated responses, then
compact each block once at finalization. Thinking and tool-call blocks are
visible while streaming and remain summarized by default. Interruption marks
visible partial blocks without promoting them to provider history. `/verbose`
or the configurable global `alt+t` binding reprojects the transcript with
details. Page Up / Page Down (also `alt+k` / `alt+j`) scroll while input and a
responsive semantic indicator row remain visible below it. Default message roles
and timestamps occupy a title line above the `▏` body box instead of prefixing
content. Completed assistant responses show tok/s exactly once, on their final assistant
block, and only from provider-reported output tokens and positive native monotonic elapsed time. The boxes render
bounded headings, emphasis, inline
and fenced code, lists, quotes, and links as semantic spans. The pure `layout`
service uses the same wcwidth-style combining, modifier, East Asian wide, and
emoji ranges as the native presenter. It also owns editor grapheme boundaries,
so combining sequences, emoji ZWJ sequences, and virama-attached marks are never
split by Left, Right, or Backspace. Message spans and picker options therefore
wrap on grapheme/UTF-8 boundaries using terminal cells rather than bytes or scalar count.
Set `config.messages.markdown` to `false` (or `plain` to `true`) for literal text.
`config.messages` also controls initial `verbose` and structural `redact_keys`,
`max_string`, `max_items`, and `max_depth` limits. Headless output remains plain.
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

`indicators` is a focused registry for semantic status values. Features call
`misa.reg_indicator({id,label?,icon?,hotkey?,value=function(db)...end})`.
`config.status.indicators` selects order, label/icon representation, an optional
structured keybinding reminder, and drop priority. The common component styles
`indicator.label`, `indicator.value`, and `indicator.hotkey` independently and
removes low-priority items at narrow widths. Standard registrations cover
activity, model, effort (with its cycle hotkey), session usage, context usage,
and transcript detail (`summary`/`verbose`). Root composition stays generic.

`request_options` derives request readiness and selected option values entirely
from the active model's `api.request_options` metadata. Required options without
values block a request before the user turn is recorded and emit a structured
harness problem. `effort` is the reasoning-effort affordance: `/effort` lists
only the selected model's declared choices, and the configurable global
`cycle_effort` binding (default `alt+e`) cycles those choices. Model switches
retain an equivalent value when supported, otherwise use the new model default;
models without reasoning support simply expose no effort value.

`choices` owns generic choice sessions: query text, fuzzy filtering, highlight,
view navigation, and acceptance are shared by inline slash completion and
searchable overlays. Items have semantic display fields (`label` and
`description`) plus an optional `search` string or string array, so aliases and
model names can remain searchable without being rendered. Sessions project the
same marker/hotkey/label/value/description/selected/active row model on both
surfaces.

Choice views are immutable implementations registered with
`misa.reg_choice_view`. The built-ins are `all`, `favorites`, `frecency`, and the
opt-in `slash-prefix` tree. `config.choices.purposes` maps a purpose to its
ordered visible views; the active view is always the leftmost and Right Arrow
rotates the order. The tree is available to model sessions through view
replacement but is not an extra default model column. It groups prefixes at
common `/`, `.`, and `:` delimiters, searches every descendant's full hidden
value/label/search fields, enters a unique group while typing, and makes
Backspace return to the parent node atomically when the node query is empty.

Inline and overlay sessions resolve the same `keybindings.choices` actions and
positional banks. Overlay panel count, wrapped row viewport, and hotkey targets
come from one shared choice-layout projection used by both picker input and
`picker_view`. Tab or Enter accepts, only banks and slots actually rendered on
screen activate, and configured positional keys are rendered as row hints on both
surfaces. `open_overlay` (default `alt+space`) promotes the current inline
session without resetting its query, highlight, or tree node. `replace_view`
(default `alt+/`) opens the nested `picker-picker`; replacing the active view is
kept only for that choice session and never changes purpose defaults.

`dialogs` owns correlated modal/progress/alert lifecycle, actions, cancellation,
and optional text input; `dialog_view` projects that state through the replaceable
`dialog` component role. Dialog data and hints are generic—providers do not own
UI paths or rendering. `picker` is only the overlay adapter and `picker_view` owns its visual
projection. Extensions open it with semantic items and receive the chosen value
through an event. `models` owns only model catalogue, availability, and
selection policy; its `/model` command has no model-specific picker behavior.
Type in the picker to filter provider-qualified IDs, labels, or model names,
then use the arrow keys and Enter to select. Models from providers that are not logged in
are hidden. OpenAI, Anthropic, OpenRouter, and Kimi catalogues are loaded from
their model APIs after successful authentication; Misa does not maintain
fallback lists for those providers. Set a provider's `discover_models`
to `false` and provide an explicit `models` list to keep a fixed catalogue. The
editor discovers registered slash commands and argument candidates, displays
the same semantic choice rows as the overlay, and delegates filtering,
highlight, tree navigation, configured hotkeys, and acceptance to its inline
choice session. Login commands
automatically complete authentication providers contributed by enabled
provider plugins; `/model` completes the current dynamic model catalogue. Its live startup banner
shows the selected model, cumulative session tokens, and latest context use.
Interactive transcript rows are managed in a height-bounded viewport rather
than committed to terminal scrollback. `/clear` resets the in-memory conversation,
transcript, and usage. `agent` owns normalized conversation history,
repeated user turns, provider correlation, parallel tool-result collection,
normalized token usage accounting, and automatic continuation after tools.
HTTP SSE and process JSONL transports expose the same start/data/end lifecycle
and coalesce at most 32 records per queued transaction; records and complete
responses are bounded. Transport workers publish into a bounded native queue,
while the session serializes Lua transactions, redraws incrementally, and
interleaves terminal reads with stream batches. Slow policy handling therefore
applies backpressure instead of growing the session queue, and Ctrl-C can cancel
the active socket or child promptly without invoking Lua from an I/O thread.
`tool.files` registers `read_file`, `list_directory`, `write_file`, and
`edit_file`; `tool.shell` registers `shell`. They are ordinary explicit Lua
extensions and are not enabled by the harness. `misa mcp` exposes the same
Lua-registered schemas and effect translators as an MCP stdio server. The
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

## Nix

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
    json providerFake fuzzy keybindings choices preferences
    themes themeDefault animations animationDefault components layout indicators dialogs dialogView componentMessage componentEditor componentPicker componentStatus componentChrome componentDialog
    messages status picker pickerView models requestOptions effort agent editor ui
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
`misa login openrouter` uses OpenRouter's PKCE flow. In the managed UI, device OAuth remains on the alternate screen with URL, code, progress, and cancellation. OpenRouter uses a temporary loopback callback with state validation and falls back to a pasted-code dialog when callback setup or browser launch is unavailable. Use
`misa status PROVIDER` to inspect login state without exposing credential data
and `misa logout PROVIDER` to remove it. The `auth` extension provides the same
flows inside the TUI as `/login PROVIDER`, `/status PROVIDER`, and
`/logout PROVIDER`. For `claude`, all three commands are
delegated to `claude auth`. API-key and Claude login explicitly announce the temporary terminal handoff before leaving the managed screen. OAuth access credentials are refreshed from their
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
`[ "json", "protocol.anthropic", "provider.anthropic", "components", "layout", "component.message", "component.editor", "component.picker", "component.status", "component.chrome",
"messages", "models", "request_options", "effort", "agent", "choices", "editor", "ui" ]` (add `picker` and `picker_view` when
overlay choices are needed).
OpenAI and OpenRouter use `protocol.openai`; Anthropic and Kimi use
`protocol.anthropic`. ChatGPT subscription access is the separate
`provider.openai-codex` extension and its Codex Responses protocol.

NixOS, nix-darwin, and home-manager expose the same
`programs.misa.extensions` option. Nix path values select custom extensions;
bare strings must be catalog IDs. Generated config is world-readable in the
Nix store, so it must not contain secrets.
