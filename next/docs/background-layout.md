# Background layout

`DocumentUi` measures owners away from the frame on one protocol-free worker.
A host supplies a wake callback and pumps `poll_background()` on the UI thread;
nothing in `connection::Update`, the transport, or the daemon knows that layout
exists.

## Host contract

```rust
document.enable_background(waker);   // waker: Arc<dyn Fn() + Send + Sync>
// ...
document.poll_background();          // UI thread: after frames and on wakes
document.pause_background();         // parked, hidden, or suspended
document.background_work_pending();  // whether another poll can do work
```

- The waker may run on the worker thread. It must only request a UI poll (the
  native window passes `Window::request_redraw`), never touch the document.
  Wakes are coalesced and re-arming the same waker is a no-op, so a host that
  re-arms on every poll cannot feed a redraw loop.
- `poll_background` installs results and admits new work in bounded slices:
  two owners in flight, eight capture attempts, 512 already-measured flow
  steps, and 192 KiB of serialized snapshot input per poll.
- Installing a result never changes a frame. Everything measured is offscreen
  by construction, so a host may skip painting entirely on a wake that only
  carried background results. `misa-skia` does exactly that: no scene build,
  no `present`, no swapchain traffic. Because every content change marks a
  paint, the presented frame is current whenever a wake is skipped; a bounded
  failsafe repaints anyway after 32 consecutive skipped wakes for platforms
  that do not preserve presented buffers.

## What is measured

One atomic owner at a time — a node, a list item, a table row, a trailing
fragment, or a stream projection — through the production `LayoutBuilder`, in
a private UI. The input is an immutable owner snapshot whose serialized size
is counted _before_ it is cloned and capped at 64 KiB. An owner over budget
returns `SnapshotError::Oversize` and stays exact-on-demand for the frame; a
poll whose shared capture budget is spent retries the owner later. Editor
drafts, disclosure state, table alignment, and decoded image handles ride
along. A secret editor is represented only by the bullets it paints.

The worker returns:

- an **exact height**, installed in `FlowViewport`'s measurement index at the
  same width and style generation (`install_measurement` refuses any other
  layout key), and
- the owner's **display list**, installed in `RetainedScenes` only for owners
  adjacent to the visible flow (eight per cursor segment). The scene cache is
  itself bounded (`SCENE_CACHE_OWNERS`, least-recently-used eviction), so
  prewarming a long history cannot grow memory with the document.

Heights are cheap to keep; display lists are not. A height-only result still
makes later placement exact and spare. The frame rebuilds a display list the
first time an owner is actually shown — the same work prewarm or not.

## What is guaranteed

- **Exactness**: the same `TextMetrics`, theme, drafts, and disclosure state a
  frame would use. Prewarm never changes what a frame paints; regressions
  compare the prewarmed frame's scene _and_ every owner display list against a
  cold `DocumentUi` and require them to be structurally identical.
- **Freshness**: every content change bumps a cancellation generation. Results
  measured against older content are dropped at install, and width and style
  generation are refused the same way. A partial edit keeps the sweep cursor,
  so typing cannot restart a whole-document walk; a full reset replaces reading
  order and restarts the sweep from the visible origin.
- **Tail first**: `FollowTail` walks history backwards from the first visible
  owner; anchored reading walks forwards from the last. The cursor persists
  across polls and restarts when the viewport moves.
- **Bounded work**: two owners in flight, eight capture attempts, 512 skip
  steps, and 192 KiB captured per poll, one worker per presented document. A
  paused document admits nothing and wakes nothing. Dropping a document retires
  its worker without joining the UI thread on someone else's layout. Wakeups
  are proportional to measured work: one coalesced wake per result batch and
  one per 512-step sweep slice, never one per skipped owner.

## What it is not

- Not a scrollbar: total document height stays unknown until each owner has
  been measured. No height is ever guessed or estimated.
- Not an accelerator for visible content: visible owners are measured by the
  frame that first shows them, exactly as before.
- Not connection state: layout results never enter `connection::Update`.

## Rejected designs

Two earlier prototypes were measured and reverted rather than shipped
(see `docs/pixel-retained-evidence.md`): a height-only worker whose results
saved no foreground work, and a full retained-scene prewarm whose history scan
retained unbounded display lists and drove thousands of native GPU presents.
This design separates the exact height index from the bounded scene cache and
lets a background wake advance work without presenting a frame.
