# Old plugin inventory and rendering parity

The previous system's behaviour lives in `extensions/misa/*` (Fennel). This
document maps every owner to where the rewrite keeps that behaviour, and records
where the two are not yet at parity. It is a working inventory, not an
acceptance gate; `parity.md` remains the capability-level statement.

A "plugin" here is one implementation owner under `extensions/misa/`, plus the
`standard/` wiring that installs it. The rewrite does not port the Fennel
modules: it re-expresses the behaviour in Rust against the scoped protocol and
the `data → presentation data → renderer` split. Old owners that were wiring
only have no rewrite counterpart and are marked _composition_.

## Owner map

| Old owner                                                                       | Behaviour                                                       | Rewrite                                                           | State               |
| ------------------------------------------------------------------------------- | --------------------------------------------------------------- | ----------------------------------------------------------------- | ------------------- |
| `misa.actions`, `misa.keybindings`                                              | Action routing and key interpretation                           | `misa-tui::lib` `Action`, `misa-tui::prefs` keymap                | ported              |
| `misa.agent`, `misa.agent.stream`                                               | Conversation lifecycle, stream normalization, tool continuation | `misa-session::agent`                                             | ported              |
| `misa.choices`, `.layout`, `.matching`, `.picker.*`, `.preferences`, `.preview` | Choice state, ranking, geometry, previews                       | `misa-kit::picker`, `misa-tui` picker rendering                   | partial — see below |
| `misa.clipboard`                                                                | Clipboard capability                                            | `misa-tui::clipboard`                                             | ported              |
| `misa.commands`, `.palette`                                                     | Command discovery, invocation, palette                          | `misa-session::{commands,completions}`, `misa-tui` picker         | ported              |
| `misa.compaction`                                                               | Summarize and replace history                                   | `misa-session::agent`/journal                                     | ported              |
| `misa.conversation`                                                             | Conversation fold and journal                                   | `misa-session::{journal,canonical}`                               | ported              |
| `misa.costs`                                                                    | Pricing and attempt-ledger cost                                 | `misa-kernel` attempt ledger, `misa-session::usage`               | ported              |
| `misa.dialogs`, `.render`, `.view`                                              | Dialog lifecycle and rendering                                  | `misa-session::input_requests`, `misa-tui::dialogs`               | ported              |
| `misa.editor.*`                                                                 | Input state, editing, history, attachments, images, queue       | `misa-kit::editor`, `misa-tui::{chrome,save}`, blob upload        | ported              |
| `misa.json`                                                                     | JSON at data boundaries                                         | `misa-value`, `serde`                                             | ported              |
| `misa.links`                                                                    | External links                                                  | `misa-tui`/`misa-web`                                             | ported              |
| `misa.markdown.*`                                                               | Markdown parse and render                                       | `misa-session::markdown`, `misa-render::lines`                    | partial — see below |
| `misa.models.*`                                                                 | Model discovery, effort, options, preview                       | `misa-kernel` provider facts, `misa-session::{commands,views}`    | ported              |
| `misa.protocols.*`                                                              | Wire formats                                                    | `misa-kernel::provider`                                           | ported              |
| `misa.providers.*`                                                              | Provider adapters and auth                                      | `misa-kernel::{provider,credentials,oauth,presets,mcp}`           | ported              |
| `misa.search.*`                                                                 | Search backends                                                 | `misa-kernel::search`                                             | ported              |
| `misa.selection.*`                                                              | Selection state, documents, render                              | `misa-render::select`, `misa-tui` selection                       | partial — see below |
| `misa.tools.*`                                                                  | File/shell/web tools                                            | `misa-kernel::tools`                                              | ported              |
| `misa.transcript.*`                                                             | Message model, groups, syntax, tools, viewport                  | `misa-session::views`, `misa-render::lines`, `misa-tui::retained` | partial — see below |
| `misa.ui.animations.*`                                                          | Clock-driven animation frames                                   | —                                                                 | **gap**             |
| `misa.ui.chrome`                                                                | Root header lines                                               | `misa-tui::chrome`                                                | ported              |
| `misa.ui.components.*`                                                          | Component registry and resolution                               | `misa-render::components`                                         | ported              |
| `misa.ui.layout`                                                                | Grapheme width, wrap, columns, clip                             | `misa-render::text`                                               | ported              |
| `misa.ui.status.*`                                                              | Status indicators                                               | `misa-render::components` indicators, `misa-session::indicators`  | ported              |
| `misa.ui.themes.*`                                                              | Palette, style tokens, overrides, fallbacks                     | `misa-render::theme`                                              | partial — see below |
| `misa.ui.values`                                                                | Typed fact formatting                                           | `misa-render::fact`                                               | partial — see below |
| `misa.usage.*`                                                                  | Usage capture, dialog                                           | `misa-session::usage`, reports                                    | ported              |
| `misa.standard.*`                                                               | Stock composition/registration                                  | crate `stock()`/`default()` constructors, `misa-daemon` wiring    | composition         |

## Rendering, styling, theming, scrolling

These are the areas where the rewrite is visibly behind. They are ordered by
what a person notices first.

### Scrolling and anchoring — **ported**

The old viewport (`extensions/misa/transcript/viewport.fnl`) owns a _semantic
anchor_: a source node, its part, a source offset and a row offset. When the
layout changes it relocates the anchor so the row the reader was looking at
stays put, and it reveals the focused selection by scrolling it into view.

The rewrite now ports that shape onto the retained rows
(`crates/misa-tui/src/retained.rs`). `Anchor` is the node under the top row and
its ordinal among the consecutive rows sharing it — the rewrite's `line-anchor`,
since rendered lines carry the node id rather than source offsets.
`viewport_start` is `viewport-top`: following tracks the tail, a reader's scroll
owns a row, and on a layout change `anchored_row` finds the node again instead
of trusting the row number. `Screen::scroll_intent` distinguishes a reader's
scroll from a layout shift under the same number, and the event loop writes the
resolved row back so deltas stay relative to what was shown. Selection reveal is
the same function's second half, gated on the head moving so a later reader
scroll is not fought on every repaint. A layout epoch keeps an ordinary repaint
from re-walking the document.

Not carried over: the old anchor's byte-level source offsets (the rewrite's
lines do not track them) and its exact "resume follow at the bottom" rule.

### Animations — **ported**

`extensions/misa/ui/animations/*.fnl` is now `crates/misa-render/src/animations.rs`:
a `Registry` of `Animation { frames, still }` with `pulse`, `spinner` and
`static`, a `frame(enabled, tick)` that wraps and falls back to the still frame,
and an `is_moving` predicate. `components::Selection` gained an `animation` id
(the shipped activity indicator selects `pulse`), and `misa-tui/src/event_loop.rs`
advances it from the redraw tick instead of holding a `FRAMES` constant. What
remains from the old module is the per-role selection and the enabled/interval
config; both are client preferences, so they belong with the theme overlay work
below rather than in the registry.

### Theme vocabulary — **ported**

The old theme has a palette of named colours, style-token overrides, required
token fallbacks, and a fixed 88-token vocabulary
(`extensions/misa/ui/themes/styles.fnl`). The rewrite has a role-addressed
theme with prefix fallback, a syntax-token map and a state map
(`crates/misa-render/src/theme.rs`), which is the better model: a plugin role
resolves without a theme naming it.

Ported: global emphasis/quote modifiers (`bold`, `italic`, `underline`,
`strikethrough`, `code`, `link`, `quote`) that `lines::span_style` and quote
rendering overlay on a node's own role; the markdown vocabulary
(`markdown.heading.1..6`, `markdown.list.marker`, `markdown.rule`,
`markdown.table.header`, `markdown.table.border`, `markdown.code.label`,
`markdown.code.border`) through a node-specific-then-global lookup;
`diff.added`/`diff.removed`; both `keyword` and `syntax.keyword` spellings;
`choice.preview`; all three theme choices in the TUI action palette;
`Theme::compose`, the ordered merge the old system called compose; and the
override half of `config.styles`/`config.palette`:

- `Palette` is the named-colour table a theme is built from (`accent`, `muted`,
  `error`, `keyword`, the surfaces, …), and `Theme::from_palette` builds every
  role from it;
- the light theme is the previous system's explicit light palette, not dark
  colours with their foregrounds dimmed;
- `ThemeOverrides` carries `palette`, `roles`, `tokens` and `states` patches;
  a palette patch rebuilds the theme so one name recolours a whole family, then
  the role patches win. They live in `Prefs::theme_overrides` and apply to
  whichever base theme is selected.

`hover`/`disabled` were applied by the old component resolver; the terminal has
no hover and the browser has CSS, so they stay there rather than as dead roles.

### Markdown and code — **ported**

Ported: headings (ATX and setext), lists (nested, with task boxes), quotes,
rules, fenced code, diffs, pipe tables, and inline
strong/emphasis/strong-emphasis/strikethrough/code/links. The previous system's
`markdown.render` layout is now in `misa-render::lines`:

- `allocate_columns` replaces the shrink-widest loop for tables — at least three
  cells and at most forty, grown or narrowed to fill the viewport;
- a table too narrow to draw stacks its fields as `label: value`, as the old
  renderer did;
- task lists carry `markers` beside `items` (serde-defaulted), so the state is a
  fact on the tree rather than a glyph the renderer re-reads; clients draw the
  Unicode ballot box — `☑` U+2611 with check, `☐` U+2610 empty — and the field
  is validated to have one entry per item or none;
- `***strong emphasis***` is `SpanKind::StrongEmphasis`, one span a client marks
  bold and italic rather than strong around a stray `*`;
- prose code blocks (`.markdown.` roles) carry a numbered gutter in
  `markdown.code.border`; a diff numbers from its hunk headers (removed lines
  name the old file, added and context lines the new one, metadata names
  neither), and a tool's own code view is left unnumbered.

Not carried over: reference-style links and images (a markdown image is a URL,
not the blob hash [`Kind::Image`] carries, so the alt/link text is kept).

### Choices — **ported**

`choices.matching` is the word-based rule in `misa-kit::picker::match_score`:
the query splits into words, each must match, an exact hit beats any subsequence
and an earlier exact hit beats a later one, and the detail field joins value and
label in the searchable text. `choices/layout`'s `layout.columns` is now
`misa-render::text::columns`, used by the picker to give the leading panels the
remainder of an odd split instead of dropping a cell. The picker already had
views, frecency, shortcuts, completion, preview, overflow and hints.

### Values — **ported**

`values.builtins` is now the registry in `crates/misa-render/src/fact.rs`.
`Registry::default()` is the shipped formatter table, `stock()` is the one the
renderers resolve through, and an unknown role still falls back to the value's
own text. Added: `text`, `boolean`, `number`, `datetime` (RFC 3339 or Unix
seconds, with an optional `prefix`/`relative_to`/`fallback`), `sequence`, and
`unavailable`, plus the RFC 3339 parser and `relative_time`. Local time is
carried as `utc_offset` seconds in the fact, so the formatter stays pure and the
session or client supplies the zone instead of the renderer reading a host
clock.

### Editor — **ported**

`ui/layout.fnl`'s grapheme segmentation is now in `misa-kit::editor`
(`boundary_at_or_before`, `previous_boundary`, `next_boundary`): backspace,
delete, left/right and the up/down column all move by grapheme cluster, so a
combining mark or a joined emoji is one keypress rather than several. The old
`take`/`fit`/`wrap-input` equivalents are the rewrite's `text::{clip, pad}` and
the composer's own cursor-centred viewport.

### Syntax highlighting — **ported, client-side**

Highlighting is appearance, so it is now a client concern and the session no
longer carries it. The protocol was simplified to match: `Kind::Code` is just
`{ lang, text }` — `lang` only when the author fenced one — and `Capture` and the
unused `SpanKind::Token` are gone from `misa-proto`.

The highlighter lives in a new client crate, `misa-syntax`. It walks a
tree-sitter parse and returns generic classes (`keyword`, `string`, `number`,
`type`, `function`, `property`, `comment`, `escape`, …) that the client's theme
tokenizes. It is query-free, as the previous system was, so a new grammar needs
no query file; the shipped grammars are rust, python, javascript/typescript/tsx,
json, bash, go, c, cpp, toml and yaml. Parsing is bounded (`MAX_SOURCE_BYTES`,
AST depth and node counts) and an unknown or over-large source is plain text
rather than partial colour. `language_for_path` maps a tool argument's path
extension to a language for code that arrived without an authored label.

Which clients use it is their choice: the terminal's linear renderer does, the
browser does not yet (it renders plain until it gets its own), and a client that
never wants to parse just draws the text.

### Region composition and truncation — **partial**

Ported loosely: header, transcript, dock (queue/attachments), composer,
inline completions, status, modal/overlay arbitration. Not ported: the old
`exclusive`/`overlay`/`dock` layer declarations with priorities, and head/tail
truncation with an "N lines hidden" notice. The rewrite hardcodes the layout in
`retained::frame_with`; a small region/priority model would make new panels and
contributions stop competing for hardcoded rows.

## Rendering boundary

The session emits semantics; each client renders. It is now structural:

- `misa-render` holds only medium-agnostic primitives — `text`, `theme`,
  `fact`, `animations` — and nothing that assumes cells.
- `misa-lines` holds the linear renderer (`lines`, `components`, `select`) and
  its `Line` type, used by the terminal and pipe clients.
- `misa-syntax` is client-side highlighting; `misa-lines` and `misa-skia` use it,
  the browser does not yet.
- `misa-skia` renders a native scene directly from the tree, with its own leaf,
  component, table and code handling.

Nothing in `misa-session` or `misa-kernel`'s normal dependency graph reaches a
render or syntax crate; the client crates link no session or kernel.

Skia draws a message as a card — one surface spanning the whole block and one
full-height rail — in both the headless scene and the interactive window. Its
activity indicator animates on a client frame clock (`misa-render::animations`),
and the window waits rather than polls when nothing is running. What it still owes
its medium: sidebars, proportional type, and sideways-scrolling code. The flow is
still column-and-row text; the structure no longer forces the terminal model on it.

## Ordering

1. `misa-skia` GUI layout: cards/rails with real message extents, sidebars,
   proportional type, sideways code, animation.
2. **Tool-code highlighting** — `misa-syntax::language_for_path` exists, but the
   linear renderer has no parent context to read a tool's `path` argument, so only
   fenced markdown is highlighted today. Annotating a client's own tree copy is the
   client-side fix.
3. **Region/priority model and head/tail truncation**, when a surface needs
   them.
4. **Structured selection documents** (`selection/document.fnl`) — section, list
   and table navigation rather than row/node-at-a-time.
