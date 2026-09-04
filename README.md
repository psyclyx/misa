# misa

misa is a small, policy-free coding-agent harness built with Zig 0.16 and the
system LuaJIT. It supplies extension loading and composition, not an agent:
there are no built-in providers, prompts, tools, loops, or default extensions.
Those decisions belong in Lua extensions and configuration.

## Build and run

LuaJIT development headers and `pkg-config` must be available.

```sh
zig build
zig build test
zig build run -- --config ./misa.json extra-argument
```

A configuration path is mandatory, either through `--config PATH` or the
`MISA_CONFIG` environment variable:

```json
{ "extensions": ["/path/to/first.lua", "/path/to/second.lua"], "config": {} }
```

`extensions` is ordered. `config` may be any JSON value. Either top-level field
may be omitted, defaulting to `[]` and `{}` respectively. Extension paths are
interpreted by LuaJIT from misa's working directory.

misa consumes at most one `--config PATH` before a `--` terminator. All other
arguments are forwarded unchanged to extensions; arguments after `--` are
always forwarded, even if they are named `--config`. The executable name,
consumed `--config` pair, and `--` terminator are not included in
`context.argv`.

## Architecture

`src/main.zig` only handles CLI/process concerns. `src/config/root.zig` owns the
JSON envelope and deliberately does not interpret its free-form payload.
`src/lua_runtime/root.zig` owns the LuaJIT lifetime behind a narrow C shim.
These are explicit named Zig modules in `build.zig`; lower layers do not import
the CLI.

misa loads every extension before invoking callbacks. Each script must return a
table with optional `setup(context)` and `run(context)` functions. All `setup`
callbacks run in extension order, followed by all `run` callbacks in extension
order. A callback error stops execution.

The shared context contains:

- `config_json`: the free-form `config` value serialized as JSON;
- `argv`: only the forwarded extension arguments, indexed from 1;
- `misa`: the same generic composition API available as the global `misa`.

The API has two operations:

```lua
misa.register("event-name", function(value) return value end)
local ordered_results = misa.call("event-name", value)
```

Handlers are called in registration order and `call` returns their results as a
table. The table's `n` field is the handler count, and each handler's first
return value remains at its registration index. A `nil` result therefore leaves
a hole rather than shifting later results; iterate from `1` through `results.n`
instead of relying on `#results` or `ipairs` when nils are possible. Extensions
can use the normal LuaJIT standard libraries for files, processes, modules, and
other basic facilities.

## Nix

The standalone `default.nix` exports `packages`, `overlay`, `shell`, `lib`, and
NixOS/nix-darwin/home-manager modules. Dependencies are pinned with npins.

`lib.mkMisa` creates a wrapped package whose generated JSON config references an
ordered list of extension paths. Nix store references keep the complete runtime
closure available:

```nix
let
  misaProject = import ./path/to/misa { inherit pkgs; };
  configured = misaProject.lib.mkMisa {
    extensions = [ ./extensions/provider.lua ./extensions/ui.lua ];
    config = { model = "example"; nested = [ 1 true ]; };
  };
in configured
```

The three module variants share these options and install the derived wrapped
package. They import the project package with the module's `pkgs`, so using a
module does not require installing the overlay. Each module family exports both
`misa` and `default` aliases:

```nix
{
  imports = [ misaProject.nixosModules.default ]; # or darwin/homeManager
  programs.misa = {
    enable = true;
    extensions = [ ./extensions/my-extension.lua ];
    config = { };
  };
}
```

There are no default extensions; `extensions` defaults to `[]` and `config` to
`{}`.

Nix-generated configuration is copied into the world-readable Nix store. Do
not put secrets in `programs.misa.config` or `lib.mkMisa` configuration; supply
secrets at runtime through an appropriate extension-controlled mechanism.
