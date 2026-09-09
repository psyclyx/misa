# Standard library layout

Bundled modules live under `misa/`. Paths express ownership: keep code together
when it changes for the same domain reason. Fuzzy matching belongs to choice
ranking, so it lives at `misa.choices.matching`. Markdown owns parsing and
rendering under `misa.markdown`. A shared input type or component interface does
not establish shared ownership.

| Owner | Responsibility |
| --- | --- |
| `misa.standard` | Stock application data, settings, and registration wiring |
| `misa.agent` | Conversation lifecycle, response correlation, tool continuation, and stream normalization |
| `misa.models` | Model discovery and selection, request options, effort, and model previews |
| `misa.costs` | Model pricing and response cost accounting |
| `misa.usage` | Usage capture and refresh policy; its `dialog` provides the optional dashboard |
| `misa.choices` | Choice state, ranking, geometry, previews, preferences, and picker behavior |
| `misa.commands` | Command discovery, invocation, and the command palette |
| `misa.actions`, `misa.keybindings` | Action routing and contextual keyboard interpretation |
| `misa.dialogs` | Dialog lifecycle, composition, and rendering |
| `misa.editor` | Input state, editing, history, attachments, images, queued input, and presentation |
| `misa.selection` | Selection state, source documents, and presentation |
| `misa.markdown` | Markdown documents and rendering |
| `misa.transcript` | Response models, viewport, groups, syntax, and tool presentation |
| `misa.protocols`, `misa.providers` | Wire formats, transport adapters, and authentication policy |
| `misa.tools` | File and shell operations |
| `misa.clipboard`, `misa.links` | Clipboard and external link effects |
| `misa.ui` | Frame composition, layout, typed values, and shared presentation machinery |
| `misa.json` | JSON encoding and decoding at data boundaries |

A small independent owner can be one file. An owner with children uses
`init.fnl`: `misa.editor` loads `misa/editor/init.fnl`, while
`misa.editor.history` loads `misa/editor/history.fnl`. Do not add a sibling
`editor.fnl`. Use hyphens within multiword names and plural names for families
such as `providers`, `tools`, and `components`.

Use `render.fnl` for component implementations and `view.fnl` for preparing
semantic child views from feature state. A dialog renderer stays beside dialog
state. Shared primitives belong under `misa.ui.components` when their contract
is independent of a particular feature. Status consumes usage subscriptions;
it does not own usage capture. The usage dashboard can be omitted while keeping
usage lifecycle handlers.

`misa.runtime.*` names private framework modules embedded from
`src/lua_runtime/`. Those modules own transactions and subscriptions, rather
than selectable application policy.

# Implementation and composition

Implementation modules return tables of named operations and domain values.
They do not assign catalog IDs, subscribe handlers, or choose keyboard bindings.
For example, `misa.agent` exposes conversation transitions and
`misa.providers.command` exposes process request and completion operations.
These functions can be tested with explicit inputs.

Stock modules under `misa.standard` return declaration maps. They associate
implementation values with application identities and inputs:

```fennel
(local agent (require :misa.agent))

{:events {:agent/cancel {:event :agent/cancel-active
                        :priority 63000
                        :handler agent.cancel}}}
```

A declaration key identifies a registration; its `event` field selects the
incoming event. They serve different purposes and need not match. Several
independently replaceable handlers may observe the same event. Source names,
service paths, event names, and registration IDs are also separate contracts.
Moving a file does not rename its public service or event vocabulary.

[`misa.standard`](misa/standard/init.fnl) combines the stock fragments into
`{:config settings :definitions catalogs}`. Its
[`settings`](misa/standard/settings.fnl) contains runtime defaults. Individual
fragments remain available through ordinary `require`:

- `misa.standard.editor` contains editor wiring.
- `misa.standard.providers.openai` contains OpenAI provider wiring;
  `misa.standard.protocols.openai` contains the shared protocol catalogs.
- `misa.standard.agent.stream` selects stream normalization independently of the
  conversation lifecycle.
- `misa.standard.presentation.*` contains shared presentation composition.
- `misa.standard.tools.files` contains file tool registrations.

Importing a fragment does not install it. Its entries can be selected, combined,
changed, or omitted with ordinary table operations. The complete application
crosses one validation/install boundary, after which its catalogs are sealed.
There is no module constructor protocol or separate override language.

Copy stock data before editing it so another import remains unchanged:

```fennel
(local app (misa.snapshot (require :misa.standard)))
(local replacement (. (require :my.editor) :input))

(tset app.definitions.components :default.editor.input replacement)
(tset app.definitions.keybindings :global/toggle_verbose :default ["alt+v"])
(tset app.definitions.commands :/clear nil)
app
```

`replacement` is a component record such as `{:render render-input}`. Existing
keys replace values, new keys add values, `nil` removes them, and `[]` is an
empty array. `misa.snapshot` copies nested tables and retains function values.
Database patch controls such as `misa.delete` belong to state transitions, not
application composition.

Keep request URLs, timeouts, persistence preferences, and other runtime options
in `config`. Event adapters receive `cofx.config`; service and effect adapters
can read the installed immutable settings with `misa.configuration()`. Pass the
relevant data into implementation functions. Model entries, auth profiles,
keybinding arrays, and renderer implementations belong in declaration catalogs.
Runtime selections such as the active theme or component role remain feature
state and can still be persisted or changed by events.

The Nix `standardExtensions` catalog mirrors source paths. Its `init` attribute
names an owner's entrypoint: `standardExtensions.misa.standard.agent.init`
evaluates to `"misa.standard.agent"`. See the
[application contract](../README.md#event-coeffect-effect-and-view-contract) for
catalog shapes and configuration examples.

# Fennel conventions

Follow the [Fennel style guide](https://fennel-lang.org/style) and format sources
with `fnlfmt`. Use `local` for module bindings and `let` inside functions. Bind
related values together; use `var` only for values that change. Prefer collection
expressions for transforms and Fennel control flow to raw Lua. Keep comments for
rationale that code cannot express.

Every module returns a table. Give each public function a short docstring whose
first sentence explains its operation; separate further details with a blank
line. Preserve established host data keys, use kebab-case for Fennel names, and
use `?` for predicates.

Substantial behavior belongs in named functions outside wiring tables. Pass
configuration and other dependencies explicitly; small adapters can make their
association clear. Do not expose private helpers as runtime services merely to
test them. Stateful parser and cache objects are appropriate when that state is
the abstraction being created; they do not imply a construction lifecycle for
ordinary modules.

Direct policy tests exercise state transitions and request construction without
installing an application. Integration tests cover stock wiring and host
contracts. `tests/extension-style.fnl` checks module exports, public docstrings,
and lexical/control-flow conventions; these checks supplement review.
