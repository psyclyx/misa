# UI worklist

Tracked, unfixed. Nothing here gets a workaround: if an item needs a refactor
to stay simple, do the refactor; if the honest fix is deletion, delete.

## Scroll bars

- [ ] **Wrong space accounting.** The bar is painted _over_ the transcript at
      `width - 9` instead of reserving its own column, and the thumb proportions
      need checking against real content. Reserve the column in the layout and let
      the content width shrink accordingly.
- [ ] **Hide when not needed.** No overflow, no bar. (Today it appears whenever
      the height index is complete, even for content that fits.)
- [ ] **Less rectangular.** Rounded track/thumb. This wants a real rounded-rect
      in the paint model (`Op` gains a radius, or a rounded-rect op) — drawn
      geometry, not nine-patch hacks.

## Text size

- [ ] **C-+ / C-- / C-0 change the base text size.** `FONT_SIZE` is a const
      threaded through the pixel document; it becomes owned state (`DocumentUi`
      carries the size; widgets already take `font_size`). Host wires the keys;
      layout remeasures at the new size. Persist the choice like appearance.

## Thinking blocks

- [ ] **Drop the "thinking" label from the collapsed state entirely.** The
      short form is the trace preview; nothing names the block type in its text.
- [ ] **Click anywhere on the block toggles it.** The whole block is the
      disclosure target when collapsed; when open, clicking its body toggles it
      back (text selection inside thinking is secondary — reading tool).
- [ ] **Lighter text** than regular prose (the thinking surface is already
      distinct; its type should be too).

## Markdown

- [ ] **Headings render larger** (per level, measured — the layout already
      measures whatever size it is given).
- [ ] **Clicking a link opens it.** Links become real controls with an
      open-URL command; the host opens it (never the toolkit).
- [ ] **Cursor state follows the target** (link, button, text): the hit model
      carries a cursor kind; the host maps it to the platform cursor.
- [ ] **Link target shown on hover** — a hint line near the status bar or a
      tooltip, not painted into the document.
- [ ] **`![]()` images render.** Inline markdown images reach `Kind::Image`
      and the decoded-image path (including the privacy/limits already in place).
- [ ] **Image chrome is minimal.** No metadata row, no big button under the
      image: metadata on hover (tooltip), save/copy in a context menu. Deletes the
      current image action block where it can.

## Tool calls

- [ ] **Show what the call was and did**, not just its name: arguments as a
      short summary and the result/state in the collapsed row; the long forms stay
      in the expansion.

## Blocks

- [ ] **Thinking spacing matches every other block.** Whatever the disparity
      is, it comes from one place: block chrome is shared, so measure and fix it
      there.
- [ ] **Thinking is just a block.** No grouping with the text or tool calls
      that follow it — in the pixel UI _and_ the TUI. The TUI currently nests
      thinking blocks inside other blocks; the tree/indentation is wrong there.
- [ ] **Blocks get their own background** (slightly distinct from the page).
- [ ] **More padding, left side especially.** Card padding lives in one
      constant block (`CARD_PADDING_*`); fix it there and let every block follow.

## Status bar and turn markers

- [ ] **Turn markers match old misa.** Discover the old behavior first, then
      implement it exactly once.
- [ ] **tok/s shown** (tokens per second), with whatever rate the session
      reports — semantic data, not painted guesswork.
- [ ] **The Skia status bar is a bar, not a text row.** Real chrome: its own
      surface, segment spacing, stable columns.

## Picker / autocomplete

- [ ] **Model selection works in Skia** by name _or_ id; you must never have to
      know an id to pick a model.
- [ ] **The TUI picker keys on the id** (displaying the name is fine; the id is
      what is sent).
- [ ] **Completions open as you type** — no manual popup step in Skia.
- [ ] **The picker serves every completable place**, not only top-level
      commands: model fields, destinations, and other id/name choices.

## Rules of engagement

- One owner per behavior: shared chrome in the shared place (the fit/flow/
  scrollbar modules), semantic text in the semantic layer, platform effects
  (opening a URL, cursors, toasts) in the host.
- Prefer deleting a wrong path over adding a parallel one.
- Every item above ends with a test that would have caught it.
