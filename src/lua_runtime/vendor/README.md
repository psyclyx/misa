# Bundled Fennel compiler

`fennel.lua` is the unmodified generated library from Fennel 1.6.0,
MIT licensed (see `FENNEL-LICENSE`). It is embedded in the executable so
loading Fennel extensions needs no separately installed compiler.

Upstream: https://github.com/bakpakin/Fennel
Tag: `1.6.0`
Commit: `7b2192e405c3b7474acc25135b8aafd2800e131d`

To reproduce, check out that commit and run `make fennel.lua LUA=luajit`.
Copy `fennel.lua` and `LICENSE` here. The compiler itself is written in
Fennel; this generated Lua artifact bootstraps it on the embedded LuaJIT VM.
