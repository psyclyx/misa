# misa

misa is a small coding-agent harness built with Zig 0.16 and system LuaJIT. The
core supplies ordered extension loading and composition. It has **no extensions
enabled by default**: `{}` is valid and silent.

The distribution includes three optional standard extensions:

| ID                 | Installed file         | Purpose                         |
| ------------------ | ---------------------- | ------------------------------- |
| `agent`            | `agent.lua`            | one-shot agent                  |
| `provider.fake`    | `provider/fake.lua`    | deterministic response provider |
| `provider.command` | `provider/command.lua` | trusted local command provider  |

These remain extensions, not a second plugin system. Select standard extensions
by exact bare ID in the existing ordered `extensions` list. An entry containing
`/`, or ending in `.lua`, is a literal custom path and is left unchanged. Any
other bare value is an unknown-ID configuration error.

## Build and run

LuaJIT development headers and `pkg-config` must be available.

```sh
zig build
zig build test
zig build test-nix # explicit Nix evaluation suite; requires Nix
zig build -Doptimize=ReleaseSafe
zig build run -- --config config/default.json hello
```

`zig build` installs the complete `extensions/` tree under
`share/misa/extensions`. Bare IDs resolve first under `MISA_EXTENSION_DIR`, when
set, and otherwise under `../share/misa/extensions` relative to the running
executable. `zig build run` and source-layout integration tests set
`MISA_EXTENSION_DIR` to the source tree. `zig build test` also installs into its
build prefix and automatically verifies executable-relative lookup with the
override unset. Literal paths are interpreted by LuaJIT from misa's working
directory.

A configuration path is mandatory through `--config PATH` or `MISA_CONFIG`:

```json
{
  "extensions": ["provider.fake", "agent", "./local/report.lua"],
  "config": {
    "agent": { "provider": "fake" },
    "providers": { "fake": { "responses": ["hello\n"] } }
  }
}
```

Both fields may be omitted, defaulting to `[]` and `{}`. `extensions` order is
preserved. misa consumes at most one `--config PATH` before a `--` terminator.
All other arguments are forwarded unchanged; arguments after `--` are always
forwarded, even if named `--config`.

The checked-in `config/default.json` explicitly selects `provider.fake` and
`agent`; it is an exact smoke-test example, not a default imposed by misa.

## Extension API

Every script returns a table with optional `setup(context)` and `run(context)`
functions. All setup callbacks run in extension order, then all run callbacks.
An error stops execution. Context contains:

- `config`: the decoded free-form JSON value;
- `config_json`: the same value serialized as JSON, retained for compatibility;
- `argv`: forwarded arguments, indexed from 1;
- `misa`: the global composition API.

JSON objects, arrays, strings, booleans, integers, floats, and null are converted
recursively. Every JSON null, including nested nulls, is the stable
`misa.json_null` singleton. LuaJIT numbers are doubles, so large JSON integers
can lose precision. Lua tables also cannot intrinsically distinguish an empty
JSON array from an empty object after decoding; use `config_json` if either
distinction matters. Conversion is limited to 128 nested array/object levels;
deeper configuration is rejected during LuaJIT initialization rather than
risking native or Lua stack exhaustion.

```lua
misa.register("event-name", function(value) return value end)
local count = misa.handler_count("event-name")
local results = misa.call("event-name", value)
```

Handlers run in registration order. Results have an `n` handler count; nil
results leave holes, so iterate `1` through `results.n` rather than using `#` or
`ipairs` when nil is possible.

## Standard extension contracts

### `agent`

The one-shot agent requires `config.agent.provider` and at least one forwarded
prompt argument. It joins prompt arguments with spaces, optionally includes the
string `config.agent.system_prompt`, and calls exactly one
`provider.<id>.complete` handler with a request table. The handler must return
`{ text = string }`; the agent prints that text, adding a trailing newline only
when the response lacks one, and exits. There is no tool loop.

### `provider.fake`

This registers `provider.fake.complete`. Each call returns the next string from
`config.providers.fake.responses`; exhausting the configured responses is an
error. It is intended for deterministic examples and tests.

### `provider.command`

This registers `provider.command.complete` and requires a nonempty array of
nonempty, NUL-free strings at `config.providers.command.argv`. It rigorously
POSIX-shell quotes every configured argument and appends the request prompt as
one final, quoted argument. It captures `io.popen` stdout and reports both read
and close failures.

The configured executable and its environment are trusted. This provider does
not create a security boundary and should not run untrusted commands. It has no
HTTP transport or native process abstraction.

## Nix

The standalone `default.nix` exports packages, overlay, shell, `lib`, modules,
and `standardExtensions`. `lib.standardExtensions` has values convenient with
`with`:

```nix
let
  misaProject = import ./path/to/misa { inherit pkgs; };
  configured = misaProject.lib.mkMisa {
    extensions = with misaProject.lib.standardExtensions; [
      providerFake
      ./extensions/my-extension.lua
      agent
    ];
    config = {
      agent.provider = "fake";
      providers.fake.responses = [ "done\n" ];
    };
  };
in
configured
```

`mkMisa.extensions` accepts an ordered mixture of standard ID strings and Nix
path values. Bare strings are validated against the catalog; custom extensions
must be path values and are coerced through Nix interpolation to store paths
with closure references. The wrapped package sets
only `MISA_CONFIG`; standard IDs resolve from the shipped files installed in
the package.

NixOS, nix-darwin, and home-manager share the same single
`programs.misa.extensions` option and the same catalog validation:

```nix
{
  imports = [ misaProject.nixosModules.default ]; # or darwin/homeManager
  programs.misa = {
    enable = true;
    extensions = with misaProject.lib.standardExtensions; [
      providerCommand
      agent
    ];
    config = {
      agent.provider = "command";
      providers.command.argv = [ "/trusted/path/to/provider" "--mode" "plain" ];
    };
  };
}
```

No module or wrapper enables extensions by default. Generated configuration is
world-readable in the Nix store; do not put secrets in it.

## Architecture

`src/main.zig` owns CLI/process concerns and standard-ID resolution.
`src/config/root.zig` owns the JSON envelope. `src/standard_extensions/root.zig`
owns the exact catalog and resolver. `src/lua_runtime/root.zig` owns LuaJIT and
uses its C API directly with checked stack invariants. They are explicit named
modules in `build.zig`.
