# UI worklist

Tracked, unfixed. Nothing here gets a workaround: if an item needs a refactor
to stay simple, do the refactor; if the honest fix is deletion, delete.

Diagnoses below come from rendering the real session view to PNG (the probe in
`misa-skia-paint/tests/parity.rs`, currently ignored).

## Scroll bars

- [ ] **Wrong space accounting.** The bar is painted over the transcript at
      `width - 9` instead of reserving its own column. Reserve the column in
      the layout and let the content width shrink accordingly.
- [ ] **Hide when not needed.** No overflow, no bar. Today it appears whenever
      the height index is complete, even for content that fits.
- [ ] **Less rectangular.** Rounded track/thumb. This wants a real rounded
      rectangle in the paint model (a radius on the rect op) — drawn geometry,
      not nine-patch hacks.

## Layout

- [ ] **Transcript is not clipped to its area.** A row straddling the boundary
      paints over the pinned composer (probe: the `Message` label over `echo`).
      The flow content clips to its reserved region.
- [ ] **Huge empty space beneath the status bar.** The host reserves a third of
      the window (`panel_height = height / 3`) for the status panel's one row.
      Size the panel region to its content; collapse it when there is none.

## Text size

- [ ] **C-+ / C-- / C-0 change the base text size.** `FONT_SIZE` is a const
      threaded through the pixel document; it becomes owned state (DocumentUi
      carries the size; widgets already take `font_size`). Host wires the keys;
      layout remeasures at the new size. Persist the choice like appearance.

## Thinking blocks

- [ ] **Drop the "thinking" label from the collapsed state entirely.** The
      short form is the trace preview; nothing names the block type in its
      text. (The pixel preview is clean now; check the TUI too.)
- [ ] **Click anywhere on the block toggles it.** The whole block is the
      disclosure target when collapsed; when open, clicking its body toggles it
      back. Text selection inside thinking is secondary — it is a reading tool.
- [ ] **Lighter text** than regular prose. The thinking surface is distinct;
      its type should be too.

## Markdown

- [ ] **Headings render larger** (per level, measured — the layout already
      measures whatever size it is given). Today `# A heading` is body size.
- [ ] **Clicking a link opens it.** Links become real controls with an
      open-URL command; the host opens it (never the toolkit).
- [ ] **Cursor state follows the target** (link, button, text): the hit model
      carries a cursor kind; the host maps it to the platform cursor.
- [ ] **Link target shown on hover** — a hint near the status bar or a tooltip,
      not painted into the document.
- [ ] **`![]()` images parse at all.** Today `![alt](hash)` arrives as literal
      text (`!alt text`). Parse inline images in `misa-markdown` and reach
      `Kind::Image`.
- [ ] **Image chrome is minimal.** No metadata row, no big button under the
      image: metadata on hover (tooltip), save/copy in a context menu. Deletes
      the current image action block where it can.

## Tool calls

- [ ] **Show what the call was and did**, not just its name: arguments as a
      short summary and the result/state in the collapsed row; the long form
      stays in the expansion. Today the name is painted twice (title row plus
      summary row).

## Blocks

- [ ] **Thinking spacing matches every other block.** Whatever the disparity,
      it comes from one place: block chrome is shared, so measure and fix it
      there.
- [ ] **Thinking is just a block.** No grouping with the text or tool calls
      that follow it — in the pixel UI and the TUI. The TUI nests thinking
      blocks inside other blocks; the tree/indentation is wrong there.
- [ ] **Blocks get their own background** (slightly distinct from the page).
- [ ] **More padding, left side especially.** Card padding lives in one
      constant block (`CARD_PADDING_*`); fix it there and let every block
      follow.

## Status bar and turn markers

- [ ] **Turn markers match old misa.** Discover the old behavior first, then
      implement it exactly once.
- [ ] **tok/s shown** (tokens per second), with whatever rate the session
      reports — semantic data, not painted guesswork.
- [ ] **The Skia status bar is a bar, not a text row.** Real chrome: its own
      surface, segment spacing, stable columns.

## Picker / autocomplete

- [ ] **Model selection works in Skia** by name or id; you must never have to
      know an id to pick a model.
- [ ] **The TUI picker keys on the id** (displaying the name is fine; the id is
      what is sent).
- [ ] **Completions open as you type** — no manual popup step in Skia.
- [ ] **The picker serves every completable place**, not only top-level
      commands: model fields, destinations, and other id/name choices.

## Rules of engagement

- One owner per behavior: shared chrome in the shared place (the fit/flow/
  scrollbar modules), semantic text in the semantic layer, platform effects
  (opening a URL, cursors, tooltips) in the host.
- Prefer deleting a wrong path over adding a parallel one.
- Every item above ends with a test that would have caught it.
