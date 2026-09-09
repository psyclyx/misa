# Standard library layout

All bundled application modules live under `misa/`. Their `require` names follow
their ownership paths: `misa.editor.history` loads `misa/editor/history.fnl`.
An owner with children uses `init.fnl` for its entrypoint, so `misa.editor` loads
`misa/editor/init.fnl`. There is no sibling `editor.fnl` competing with the
`editor/` directory, and no compatibility aliases for older module names.

| Owner | Responsibility |
| --- | --- |
| `misa.standard`, `misa.default`, `misa.definitions` | Application construction, stock composition, and catalog declarations |
| `misa.agent` | Agent lifecycle, model selection, request options, effort, and cost facts |
| `misa.choices` | Choice state, matching layouts, previews, preferences, and picker behavior |
| `misa.commands` | Command discovery, action routing, keybindings, and command palette |
| `misa.dialogs` | Dialog state and its view composition |
| `misa.editor` | Input state, editing policy, history, attachments, images, and queued input |
| `misa.protocols` | Wire formats and normalized streaming effects |
| `misa.providers` | Provider adapters and authentication policy |
| `misa.selection` | Selection state and source document structure |
| `misa.system` | Clipboard and external link effects |
| `misa.text` | Markdown parsing, syntax captures, and fuzzy matching |
| `misa.tools` | File and shell execution definitions, plus tool summaries |
| `misa.ui` | Frame composition, transcript, layout, typed value formatting, and tool presentation |
| `misa.ui.components` | Reusable component implementations and their composition contract |
| `misa.ui.status` | Status composition, indicators, and usage presentation |
| `misa.ui.animations`, `misa.ui.themes` | Named presentation catalogs and stock implementations |
| `misa.json` | JSON encoding and decoding at data boundaries |

Group related modules under their owner. Use plural names for families of
implementations (`providers`, `protocols`, `tools`, `components`, `themes`,
`animations`), and hyphens within multiword names (`request-options`,
`openai-codex`). Put a view beside the feature whose model it presents, such as
`misa.editor.queue.view`; put reusable rendering primitives under
`misa.ui.components`. Parsing belongs in `misa.text`, tool execution in
`misa.tools`, and transcript tool presentation in `misa.ui.tools`.

`misa.runtime.*` is reserved for private framework modules embedded from
`src/lua_runtime/`. Those modules implement host transactions and subscriptions;
they are not selectable application modules.

# Composition and names

`misa.default` contains ordinary configuration and named module descriptors.
Every stock `modules` key equals its `source` name:

```fennel
{:modules
 {:misa.editor {:source :misa.editor :priority 0}}}
```

`misa.standard.application` resolves selected sources and calls their pure
constructors before installation. Merely importing the default does not load
all those sources. Custom applications can select stock sources, provide their
own `source` names, or supply a constructor directly through `build`.
See the [application configuration documentation](../README.md#event-coeffect-effect-and-view-contract)
for catalog composition and overrides.

Source names identify code ownership. They do not determine public service
paths, event types, configuration keys, or definition IDs. For example,
`misa.agent.models` provides `misa.models.lookup`, `misa.ui.components.markdown`
provides named component definitions, and `misa.editor` handles `editor/restore`.
Keep these semantic contracts stable when moving source files. Override a
module by its `modules` key; override a particular implementation by its catalog
ID.

The Nix `standardExtensions` catalog mirrors these paths. An owner's constructor
uses the `init` attribute: `standardExtensions.misa.editor.init` evaluates to
`"misa.editor"`, while `standardExtensions.misa.editor.history` evaluates to
`"misa.editor.history"`.
