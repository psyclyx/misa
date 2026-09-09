# Standard library layout

Bundled modules live under `misa/`. Paths express the feature that owns a policy
or representation: things belong together when they change for the same domain
reason. Operating on strings, returning component definitions, or invoking a
host effect is not enough to establish shared ownership.

For example, fuzzy matching implements choice ranking, so it lives at
`misa.choices.matching`. Markdown has its own document model and rendering rules,
so `misa.markdown` owns both parsing and `misa.markdown.render`. Transcript syntax
captures and tool summaries serve transcript presentation, so they live under
`misa.transcript`. A dialog renderer belongs beside dialog state and composition;
implementing the component interface does not make it a shared UI primitive.

| Owner | Responsibility |
| --- | --- |
| `misa.standard`, `misa.default`, `misa.definitions` | Application construction, stock composition, and catalog declarations |
| `misa.agent` | Conversation lifecycle, provider correlation, tool continuation, and normalized stream effects |
| `misa.models` | Model discovery and selection, request options, effort, and model preview presentation |
| `misa.costs` | Model pricing and response cost accounting |
| `misa.usage` | Token usage capture and quota refresh policy; `misa.usage.dialog` provides the optional dashboard |
| `misa.choices` | Choice state, ranking, geometry, preview dispatch, preferences, and picker behavior |
| `misa.commands` | Command discovery, invocation, and command palette |
| `misa.actions`, `misa.keybindings` | Independent action routing and contextual keybinding features |
| `misa.dialogs` | Dialog lifecycle, composition, and rendering |
| `misa.editor` | Input state, editing policy, history, attachments, images, queued input, and their presentation |
| `misa.markdown` | Markdown documents, incremental parsing, and terminal rendering |
| `misa.protocols` | Provider wire formats |
| `misa.providers` | Provider adapters and authentication policy |
| `misa.selection` | Selection state, source document structure, and selection presentation |
| `misa.transcript` | Response and block models, viewport, group boundaries, syntax captures, tool presentation, and summaries |
| `misa.tools` | File and shell execution definitions |
| `misa.clipboard`, `misa.links` | Independent clipboard and external link effects |
| `misa.ui` | Frame composition, chrome, layout, typed values, and shared presentation machinery |
| `misa.ui.components` | Component resolution, fault boundaries, and shared content, data, button, and truncation primitives |
| `misa.ui.status` | Status composition and semantic indicator registry |
| `misa.ui.animations`, `misa.ui.themes` | Named presentation catalogs and stock implementations |
| `misa.json` | JSON encoding and decoding at data boundaries |

A small independent owner can be one file. Do not invent a `text`, `system`, or
similar umbrella to reduce the number of siblings. Actions and keybindings serve
multiple features; they are not children of commands. Models, costs, and usage
have their own contracts even though the agent contributes facts to them.
Status consumes usage subscriptions; it does not own usage capture or refresh.
The usage dashboard can be omitted while retaining those lifecycle handlers.

`require` names follow paths: `misa.editor.history` loads
`misa/editor/history.fnl`. When an owner has children, its entrypoint is
`init.fnl`: `misa.editor` loads `misa/editor/init.fnl`. There is no sibling
`editor.fnl` competing with `editor/`. An `init.fnl` exposes the owner's public
contract and, for stateful features, declares its lifecycle. It need not
automatically install every child.

Use `render.fnl` for feature-specific component implementations and `view.fnl`
for composition that prepares semantic child views from feature state.
For example, `misa.dialogs.view` composes dialog content while
`misa.dialogs.render` implements its chrome. Both stay with dialogs.
Shared primitives belong under `misa.ui.components` only when their contract is
independent of the consuming feature. Name other modules for their responsibility,
such as `matching`, `history`, or `preview`; use hyphens within multiword names.
Plural names identify families of implementations (`providers`, `protocols`,
`tools`, `components`, `themes`, `animations`).

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
`misa.models` provides `misa.models.lookup`, `misa.markdown.render` provides named
component definitions, and `misa.editor` handles `editor/restore`.
Keep these semantic contracts stable when moving source files. Override a
module by its `modules` key; override a particular implementation by its catalog
ID.

The Nix `standardExtensions` catalog mirrors these paths. An owner's constructor
uses the `init` attribute: `standardExtensions.misa.editor.init` evaluates to
`"misa.editor"`, while `standardExtensions.misa.editor.history` evaluates to
`"misa.editor.history"`. `standardExtensions.misa.markdown.render` selects the
Markdown renderer independently of its parser.
