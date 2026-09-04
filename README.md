# misa

misa is a small event-driven coding-agent harness built with Zig 0.16 and
system LuaJIT. The TUI uses only the Zig standard library. LuaJIT is the sole
non-stdlib application dependency.

No extensions are enabled implicitly: `{}` is valid and produces no output.
The shipped extensions are `provider.fake`, `provider.command`, `agent`, and
`ui`. Select them, in setup order, through the one `extensions` list:

```json
{
  "extensions": ["provider.fake", "agent", "ui"],
  "config": {
    "agent": { "provider": "fake" },
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
  ones. Base coeffects always include `config`, `config_json`, `argv`, and
  `terminal={interactive=<bool>,columns=<integer>,lines=<integer>}`. These four
  names are reserved and are fresh bounded deep clones for every transaction.
- `misa.reg_fx(type, fn)`: translates a Lua policy effect to one native effect
  or an ordered array of native effects.
- `misa.reg_view(fn)`: registers exactly one semantic projection.

Each setup callback receives its own bounded deep clone of setup context, so
mutation cannot affect a later extension. Registrations are sealed after all
setup callbacks. Recursive dispatch is not available. Every transaction is
protected by a traceback handler. Extensions
must not write or render; Lua output APIs and all dynamic loaders (`load`,
`loadstring`, `loadfile`, and `dofile`) are unavailable.

The fixed native effects are:

- `{type="dispatch", event=<table>}`
- `{type="process/run", argv={<strings>}, completion=<event type>, id=<string>}`
- `{type="terminal/read"}`
- `{type="view/commit", lines=<semantic lines>}`
- `{type="app/quit"}`

Unknown native effects fail the session. `process/run` invokes direct argv,
never a shell, captures stdout and stderr with 1 MiB bounds, and never inherits
the terminal output. Captured tabs are normalized to spaces and malformed UTF-8
is repaired before completion events are dispatched. Completion events include
`ok`, `status`, `stdout`, `stderr`, and `id`.

A view is modest semantic data:

```lua
{
  lines = {
    { spans = { { text = "working", style = "dim" } } }
  },
  cursor = { row = 1, byte = 0 } -- or {row=1,column=1}, or nil
}
```

Styles are `plain`, `dim`, `bold`, `accent`, `user`, `assistant`, and `error`.
Text must be valid UTF-8 and may not contain controls, ESC, CR, or LF. Cursor
rows and columns are one-based. The alternative zero-based `byte` is an UTF-8
boundary in the concatenated spans of that row; Zig converts it to a terminal
cell column and clamps it to the presentable width. A column cursor may address
a rendered cell or the valid insertion endpoint immediately after the row, but
not space beyond it. Exactly one of `byte` or `column` is required. Zig owns
wrapping and ANSI mapping; noninteractive output
strips styles.

## Terminal architecture and limitations

`src/session/root.zig` owns a non-reentrant FIFO event loop. Lua owns canonical
application state. A transaction is fully validated, then its pending semantic
view is successfully presented, then Lua policy state is committed, and only
then are the prevalidated effects executed. Policy changes are rollback-safe
through validation and presentation. A native side-effect or I/O failure after
commit is fatal and is not generally rollbackable. `src/terminal/root.zig` owns tty/raw-mode
lifetime, incremental input decoding, and all stdout writes. Raw mode is always
restored with `defer`.

The inline presenter never enters the alternate screen and never clears the
screen or scrollback. Immutable commits use real newlines and are forgotten.
Only the mutable frame at the bottom is erased/repainted with CR, relative
movement, and per-row erase-line sequences; final cleanup erases only those
owned rows. Live logical lines are conservatively clipped to one physical row
before the final terminal column, while immutable commits may wrap naturally.
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
timeout, and POSIX signal actions restore it before chaining/defaulting fatal
and termination signals. Bracketed-paste markers are decoded, but misa does not
enable that mode because Zig exposes no portable async-signal-safe POSIX write.

## Standard extensions

`agent` stores prompt/status/response/error in `db` and performs one completion
(no tool loop). `provider.fake` keeps its response cursor in `db`.
`provider.command` translates to direct-argv `process/run`. `ui` owns a
multiline UTF-8 editor with a byte cursor, insertion, backspace, and left/right
movement, plus semantic projection. It commits the final result to scrollback
and quits. Explicit argv works in plain/non-TTY mode; an interactive invocation
accepts one submission.

## Nix

`default.nix` exports the package, overlay, shell, modules, `lib`, and
`standardExtensions`. Raw and Nix configurations use the same ordered list:

```nix
let p = import ./path/to/misa { inherit pkgs; }; in
p.lib.mkMisa {
  extensions = with p.lib.standardExtensions; [ providerFake agent ui ];
  config = {
    agent.provider = "fake";
    providers.fake.responses = [ "done\n" ];
  };
}
```

NixOS, nix-darwin, and home-manager expose the same
`programs.misa.extensions` option. Nix path values select custom extensions;
bare strings must be catalog IDs. Generated config is world-readable in the
Nix store, so it must not contain secrets.
