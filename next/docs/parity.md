# Parity with the previous terminal client

Parity means the same capability expressed through each surface's own interaction
model. It does not require identical pixels, controls or keyboard shortcuts.
This inventory describes implemented mechanisms and known limits. Build results,
real interaction checks and packaged-artifact verification are separate evidence;
see [plan.md](plan.md). Historical verification documents do not certify a newly
changed binary or APK.

## Shared mechanisms

| Capability                               | Current mechanism                                                                                             |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Discover installed commands and queries  | Scoped query catalogs with input/result schemas; no `SessionInfo` greeting                                    |
| Command arguments and completion sources | `misa_proto::preparation::{Shortcut, Arg, Source}` with exact exported members                                |
| Resident completion                      | Observe bounded source data and filter it locally                                                             |
| On-demand completion                     | Correlated finite `Read` of a declared query with source/prefix/limit                                         |
| General installed command forms          | Shared schema preparation and authoritative action bindings                                                   |
| Private input responses                  | Restricted request projection plus generation-bound response action; generic menus exclude `Request` commands |
| Picker, editor and local command parsing | `misa-kit::{picker, editor, intent}`; picker filtering remains in the composer                                |
| Semantic formatting                      | Typed facts for money, tokens, percent, ratio, duration and bytes; surface-specific rendering                 |
| Optional status/plugin views             | Client-selected finite presentation variants and composed observations                                        |
| Reports                                  | Finite data/document reads displayed and dismissed locally                                                    |
| Wasm contributions                       | Declared roots, queries, commands, bindings, tools and request forms; journaled transaction outcomes          |
| Durable history and cost                 | Kernel conversation journal and uniquely identified attempt ledger                                            |
| Credentials                              | Kernel-owned slots, injection/refresh and restricted authorization operations                                 |
| Blobs                                    | Content-addressed store and separate validated upload/download protocol                                       |

Neither a report nor a credential form opens a shared singleton panel. One client
can hide a form, choose a different status variant or open a report without
changing another client's layout or subscriptions.

## Input and local interaction

| Capability                                                   | Implementation and boundary                                                                                                   |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| Multiline prompts, cursor/word/line movement, modes and undo | Shared editor with platform key/IME adapters                                                                                  |
| Draft restoration                                            | Client-local storage, scoped to daemon/session/presentation identity                                                          |
| Submission history                                           | Local editor history; persistent drafts do not imply a second persisted prompt-history log                                    |
| Ctrl-C                                                       | Terminal interrupt/cancel behavior remains local-key policy invoking an owner command; daemon process shutdown is independent |
| Ctrl-D with empty terminal input                             | EOF behavior in the terminal client                                                                                           |
| Interrupt and submit                                         | Correlated prompt operation; queue and cancellation semantics remain owner policy                                             |
| Bracketed paste and clipboard images                         | Terminal capability adapter; image bytes upload before their references are submitted                                         |
| Selection and copy                                           | Rendered-row/native/browser selection and local clipboard capability; the session cannot set a clipboard                      |
| Focus, disclosure, scroll and theme                          | Local state retained independently from canonical document updates                                                            |
| Private form drafts                                          | Local only, reset on request-generation/lifecycle changes; credential text is never a general command draft                   |

Keyboard instructions belong to the client. A semantic composer declares its field
and send action, not that Enter must submit on every surface. Queued prompt state
belongs to the owner; editor restoration must use an explicit correlated action,
not an unsolicited broadcast into every client's draft.

## Commands and discovery

| Capability                                     | Current implementation                                                                                            |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Slash completion and command palette           | Installed shortcut/command declarations plus local actions                                                        |
| Required-argument narrowing and Tab completion | Declared argument sources; resident filtering or finite query                                                     |
| Model and reasoning-effort selection           | Owner commands and discovered/catalogued model facts; effort choices reflect model support                        |
| Provider enumeration                           | Resident provider source, including authorization method                                                          |
| `/clear`, `/compact`                           | Owner transitions; compaction summarizes and journals guarded history replacement                                 |
| Create, close and resume                       | Daemon-scoped lifecycle commands; archive discovery works with no open session                                    |
| `/status`                                      | Finite session facts rendered in a local report                                                                   |
| `/usage`                                       | Finite `usage.presentation` document with typed money/token/quota facts; `usage.report` remains available as data |
| Usage refresh                                  | Kernel capability with coalescing, stale-response rejection and failure clearing                                  |
| `/login`, `/logout`                            | Credential operations with private key/device challenges and kernel-owned storage                                 |
| Authorization cancellation                     | Explicit owner operation cancellation; hiding the local form does not claim to cancel the kernel flow             |
| Help                                           | Local view of declarations already held by the client                                                             |
| Arbitrary contributed commands                 | Shared typed preparation; request-only responses are excluded from general menus                                  |

Command receipts distinguish immediate completion, rejection, accepted operations
and indeterminate outcomes. Print/headless prompt output follows the accepted
operation's terminal result and output presentation; it does not guess completion
from a quiet status widget.

## Rendering

| Capability                              | Implementation and remaining limit                                                                                                           |
| --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Streaming answers/thinking              | Canonical document plus checked live overlay; atomic durable settlement                                                                      |
| Thinking/disclosure                     | Semantic collapsible nodes, local expanded state                                                                                             |
| Markdown                                | Owner-side parse into headings, lists, quotes, rules and inline spans; pipe tables may remain prose                                          |
| Code captures                           | Semantic capture ranges, mapped to surface styles                                                                                            |
| Diffs                                   | Semantic diff roles and linear-renderer line roles; surface decoration is not universally identical                                          |
| Tables, meters, facts, forms and images | Semantic nodes rendered by the surface; terminal images use text/alt representation                                                          |
| Group headers/footers                   | Stable message-group identities and semantic content                                                                                         |
| Following output and scrolling away     | Client-local scroll policy                                                                                                                   |
| Retained updates                        | Stable owner/node identities, incremental operations and stream appends; no whole transcript clone per token in shared replica notifications |
| Skia cost                               | Retained layout/paint groups; monospaced text and whole-window raster repaint remain explicit limits                                         |

Status indicators are built from registered domain queries and presentation
composition. They are not separately hardcoded widgets that each frontend must
reimplement whenever the owner adds a fact. Clients can also read raw data without
selecting a presentation.

## Session, daemon and work lifecycle

| Capability                            | Current implementation                                                                                                            |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Multiple clients and multiple daemons | Independent daemon relationships and scoped observations; no implicit session attach                                              |
| Local discovery/pairing               | Client relationship manager and dedicated admission protocol; choosing a session is a separate action                             |
| Reconnect                             | Generation-fenced observation recovery; retain/recover canonical state and reset omitted live text; never replay invocations      |
| Slow readers                          | Bounded retention and repair; separate ordered observation lanes, finite-read lanes and control replies                           |
| Connection presence                   | `daemon.connections` describes authenticated daemon sockets, not supposed session ownership                                       |
| Create/list/close/resume              | Durable desired membership, unique conversation writer and fresh runtime incarnations; close awaits retirement                    |
| Conversation history                  | Journal/list/load/resume; conversation-fork UX is not supplied by this refactor                                                   |
| Subagents                             | Real child execution, correlated result/parent continuation, parent-bound or independent lifetime, cancellation and deduplication |
| Restarted work                        | Retained operation/work metadata reconciles interrupted work without effect replay                                                |
| Overview                              | Cheap activity/attention summaries with operation/request references and source availability; eventual across owners              |
| Cost attribution                      | Unique attempt accounting and deduplicated descendant-inclusive totals across retained relationships                              |
| Tool approvals                        | Configurable owner gate; tools do not run before approval when that policy is enabled                                             |
| Custom input requests                 | Declared nonsecret typed forms, permitted responder, generation, expiry and validated continuation                                |
| Admission                             | Roster/open/allow/pairing policy outside session state; revocation closes access to scoped and blob protocols                     |

Tools include file read/write/list, shell and echo; provider adapters include chat
completions, Anthropic messages, Responses and controlled scripted fixtures.
Search adapters and provider usage remain kernel capabilities. A background shell
command reports through its owned process lifecycle; conversation policy does not
pretend a returned tool receipt means the process has exited.

Relationship retention is bounded. Forgetting work removes its retained
relationship/result/deduplication metadata; the attempt ledger remains. Inclusive
overview cost is therefore not a promise of all-time billing after relationships
are forgotten. Monetary execution-budget policy and live owner migration across
daemons are separate capabilities.

## Blobs and local destinations

The kernel store names bytes by content hash. Blob transport checks declared size
and content identity, independently of scoped observations. Separate transport
prevents bulk image transfer from becoming a transcript message payload.

Terminal `/save`, browser download, Skia destination forms and Android local-save
flows ask the owner to resolve an advertised attachment, then fetch the resulting
blob. Only the requesting invocation receives its result. A local destination
never becomes a daemon path. `/attach <path>` deliberately means a path on the
daemon; uploading local bytes is a different client capability.

## Surface differences and verification

| Surface  | Local responsibilities                                                                                    |
| -------- | --------------------------------------------------------------------------------------------------------- |
| Terminal | Composer and picker, modal fields, ANSI theme, retained rows, OSC 52 copy and PTY/EOF behavior            |
| Browser  | HTML controls, DOM focus/selection, local storage, retained subtree/SSE application and browser downloads |
| Skia     | Native window/IME, local dialogs/palette, theme, hit testing, selection and retained scene groups         |
| Android  | Kotlin controls over JNI, per-view state, private forms, native storage and blob destinations             |

The shared client owns protocol lifetimes and request/form models; each surface
owns its renderer and gestures. `misa-render::select` provides selection over
rendered rows. Terminal persistence is supplied locally, not by a storage policy
inside the pure editor kit.

Verification must distinguish source tests from real product behavior:

- Workspace tests cover owner/protocol semantics and surface adapters.
- Real endpoint tests cover pairing, revocation, concurrent clients, recovery,
  large documents and independent-lane progress.
- `misa-plugin --features guest-fixture` builds and runs actual Wasm contributions,
  including command/tool sharing, durable input continuation and private recovery.
- PTY, Chromium, native X11/Wayland and Android instrumentation exercise the actual
  interaction layer; they are not replaced by protocol unit tests.
- Packaged desktop products and the exact installed APK require their own final
  gates after shared dependency changes.

See [plan.md](plan.md), [verification.md](verification.md) and the retained-renderer
evidence documents for dated results and limitations. This inventory intentionally
does not carry a test total or declare an in-progress packaging gate complete.
