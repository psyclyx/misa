# misa

misa is a small event-driven coding-agent harness built with Zig 0.16 and
system LuaJIT. The TUI uses only the Zig standard library. LuaJIT is the sole
non-stdlib application dependency.

No extensions are enabled implicitly: `{}` is valid and produces no output.
The shipped extensions include `models`, `agent`, `ui`, protocol adapters,
and fake, command, Claude Code, OpenAI, Anthropic, OpenRouter, and Kimi
providers. `tool.files` and `tool.shell` provide optional coding tools. Select
them through the one ordered `extensions` list. Protocol
adapters precede the API providers that use them:

```json
{
  "extensions": ["provider.fake", "models", "agent", "ui"],
  "config": {
    "models": { "default": "fake/default" },
    "providers": { "fake": { "responses": ["hello\n"] } }
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
selects the fake provider, agent, and UI as a runnable smoke-test profile.
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
  ones. Base coeffects always include `config`, `argv`, and
  `terminal={interactive=<bool>,columns=<integer>,lines=<integer>}`. These names
  are reserved. Configuration and argv are shared immutable inputs by contract.
- `misa.reg_fx(type, fn)`: translates a Lua policy effect to one native effect
  or an ordered array of native effects.
- `misa.reg_view(fn)`: registers exactly one semantic projection.
- `misa.reg_model(model)`: adds a provider-owned catalogue entry.
- `misa.reg_tool(tool)`: adds a semantic tool schema and its effect type.
  `misa.models()`, `misa.model(id)`, `misa.tools()`, and `misa.tool(name)` expose
  the sealed registries to policy extensions.

Registrations are sealed after setup. Recursive dispatch is unavailable. Each
transaction takes one bounded working copy of `db`, then commits it only after
its effects and view pass native validation and presentation. Extensions must
not write or render: `io`, `os`, and `print` are unavailable. Ordinary Lua
source composition (`require`, `load`, `loadfile`, and `dofile`) remains
available, while native `package.loadlib`, FFI, and JIT access are disabled.

The fixed native effects are:

- `{type="dispatch", event=<table>}`
- `{type="process/run", argv={<strings>}, completion=<event type>, id=<string>}`
- `{type="http/request", url=..., json=..., credential=..., completion=..., id=...}`
- `{type="file/read", path=..., completion=..., id=...}`
- `{type="file/write", path=..., content=..., completion=..., id=...}`
- `{type="file/edit", path=..., content=..., replacement=..., completion=..., id=...}`
- `{type="json/decode", source=..., completion=..., id=...}`
- `{type="terminal/read"}`
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

Styles are `plain`, `dim`, `bold`, `accent`, `user`, `assistant`, and `error`.
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
state. A transaction is fully validated, then its pending semantic
view is successfully presented, then Lua policy state is committed, and only
then are the prevalidated effects executed. Policy changes are rollback-safe
through validation and presentation. A native side-effect or I/O failure after
commit is fatal and is not generally rollbackable. `src/terminal/root.zig`
owns tty/raw-mode lifetime and all stdout writes; `terminal/input.zig`,
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

Portability is intentionally conservative: terminal mode uses Zig's portable
POSIX/stdlib abstractions, dimensions come from `COLUMNS`/`LINES` with an 80x24
fallback, and there is no live resize. Input reads and process execution are
synchronous in this increment. Bracketed paste, fragmented UTF-8, Enter,
backspace, arrows, Escape, Ctrl-C, and EOF are decoded. Decoder, presenter,
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

`models` owns selection state and an inline `/model` picker; providers own the
catalogue entries. `agent` owns normalized conversation history, repeated user
turns, provider correlation, parallel tool-result collection, normalized token
usage accounting, and automatic continuation after tools. `tool.files` registers `read_file`, `write_file`, and
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
Claude's existing Pro/Max credentials without copying them into Misa. `ui` owns
the multiline UTF-8 editor and semantic projection. Interactive sessions return
to the editor after each response; explicit argv remains a single headless turn.

## Nix

`default.nix` exports the package, overlay, shell, modules, `lib`, and
`standardExtensions`. Raw and Nix configurations use the same ordered list:

```nix
let p = import ./path/to/misa { inherit pkgs; }; in
p.lib.mkMisa {
  extensions = with p.lib.standardExtensions; [ providerFake models agent ui ];
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
`misa login openrouter` uses OpenRouter's headless PKCE flow. Use
`misa status PROVIDER` to inspect login state without exposing credential data
and `misa logout PROVIDER` to remove it. For `claude`, all three commands are
delegated to `claude auth`. OAuth access credentials are refreshed from their
stored refresh tokens. Credentials are
written atomically with mode `0600` beneath a mode `0700` directory. The
path is `$MISA_AUTH_FILE`, otherwise `$XDG_STATE_HOME/misa/auth.json`, otherwise
`$HOME/.local/state/misa/auth.json`. They never belong in regular or Nix
configuration. `misa login claude` delegates to `claude auth login`; Claude Code
continues to own and refresh its existing subscription credentials.

API provider lists include their protocol explicitly, for example
`[ "protocol.anthropic", "provider.anthropic", "models", "agent", "ui" ]`.
OpenAI and OpenRouter use `protocol.openai`; Anthropic and Kimi use
`protocol.anthropic`. ChatGPT subscription access is the separate
`provider.openai-codex` extension and its Codex Responses protocol.

NixOS, nix-darwin, and home-manager expose the same
`programs.misa.extensions` option. Nix path values select custom extensions;
bare strings must be catalog IDs. Generated config is world-readable in the
Nix store, so it must not contain secrets.
