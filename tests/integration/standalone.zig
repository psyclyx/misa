const Harness = @import("harness.zig").Harness;
const options = @import("integration_options");

test "selection consumes only accepted presentation geometry" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/presentation-feedback.fnl"} }, "accepted presentation feedback passed\n");
}

test "individual presentation defaults can be replaced through configuration" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/presentation-overrides.fnl"} }, "presentation overrides passed\n");
}

test "application composition replaces named definitions before one installation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/application-composition.fnl"} }, "application composition contracts passed\n");
}

test "MCP and event effects share policy and serializers return request data" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/effect-policy.fnl"} }, "effect policy contracts passed\n");
}

test "choice preview renderers receive semantic pricing before shared geometry" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/choice-preview.fnl"} }, "choice preview contracts passed\n");
}

test "provider adapters feed the common usage dashboard and selected indicator" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/usage-dashboard.fnl"} }, "usage dashboard contracts passed\n");
}

test "indicator facts declare dependencies and share open typed presentation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/indicator-values.fnl"} }, "indicator value contracts passed\n");
}

test "editor lifecycle queries preserve queued work and draft submission boundaries" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/editor-lifecycle.fnl"} }, "editor lifecycle contracts passed\n");
}

test "input layer controls share semantic components and hover resolution" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/input-layer-components.fnl"} }, "input layer component contracts passed\n");
}

test "model startup and option reconciliation are registration-order independent" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/model-startup.fnl"} }, "model startup ordering contracts passed\n");
}

test "presentation startup is owned by event handlers" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/presentation-startup.fnl"} }, "presentation startup contracts passed\n");
}

test "Claude usage control requests preserve isolation and reject stale results" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/claude-usage.fnl"} }, "Claude usage contracts passed\n");
}

test "Codex quota requests preserve account binding and normalize usage windows" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/codex-usage.fnl"} }, "Codex usage contracts passed\n");
}

test "component collections retain output and roll back speculative projections" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/component-projections.fnl"} }, "component projection contracts passed\n");
}

test "component failures preserve sibling output and recover after replacement" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/component-failures.fnl"} }, "component failure containment passed\n");
}

test "Kimi usage requests preserve credential boundaries and reject stale results" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/kimi-usage.fnl"} }, "Kimi usage contracts passed\n");
}

test "presentation memoization is isolated from committed model updates" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/subscription-transactions.fnl"} }, "subscription transaction contracts passed\n");
}

test "subscription scopes preserve nullable dependencies and speculation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/subscriptions.fnl"} }, "subscription scope contracts passed\n");
}

test "model picker controls and value-only status presentation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/model-affordances.fnl"} }, "model affordance contracts passed\n");
}

test "summarizer input preserves the main model and supports tab completion" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/model-role-input.fnl"} }, "model role input contracts passed\n");
}

test "tool presentation uses reusable components and preserves source geometry" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/tool-presentation.fnl"} }, "tool presentation passed\n");
}

test "effect-only controls and keepalive policies preserve inputs" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/control-ownership.fnl"} }, "control ownership contracts passed\n");
}

test "transcript viewports have no speculative render state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/transcript-viewport.fnl"} }, "transcript viewport contracts passed\n");
}

test "transcript deltas preserve state events and preview budgets" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/transcript-delta-state.fnl"} }, "transcript delta state properties passed\n");
}

test "agent stream reducers preserve snapshots and block identity" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/agent-stream-state.fnl"} }, "agent stream state properties passed\n");
}

test "status facts retain sharing and optional presentation boundaries" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/status-state.fnl"} }, "status state properties passed\n");
}

test "selection transitions and decoration preserve prior values" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/selection-state.fnl"} }, "selection state properties passed\n");
}

test "response cost updates retain earlier snapshots" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/cost-state.fnl"} }, "cost state contracts passed\n");
}

test "animation state and timer transition properties" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/animation-state.fnl"} }, "animation state properties passed\n");
}

test "model transitions preserve state and selection invariants" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/model-state.fnl"} }, "model state properties passed\n");
}

test "request option reads are pure and preserve false values" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/request-option-state.fnl"} }, "request option state contracts passed\n");
}

test "preference updates preserve their input state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/preferences-state.fnl"} }, "preference state contracts passed\n");
}

test "patch dispatch order and rollback" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/state-dispatch.fnl"} }, "patch dispatch contracts passed\n");
}

test "generator composition replay and shrinking" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/generators-test.fnl"} }, "generator contracts passed\n");
}

test "generated persistent patch invariants" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/patch-properties.fnl"} }, "patch properties passed\n");
}

test "persistent patch data and sharing contracts" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/state-patches.fnl"} }, "state patch contracts passed\n");
}

test "syntax transitions preserve state across streaming completions" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/syntax-state.fnl"} }, "syntax state properties passed\n");
}

test "async syntax memo preserves rollback and coalesces streaming requests" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/syntax.fnl"} }, "async syntax regressions passed\n");
}

test "component resolution preserves semantic cache ownership" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/component-resolution.fnl"} }, "component resolution regressions passed\n");
}

test "long messages preserve text and Markdown styling" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/long-messages.fnl"} }, "long message regressions passed\n");
}

test "image acquisition preserves state across asynchronous transitions" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/image-state.fnl"} }, "image state properties passed\n");
}

test "choice sessions and rendering preserve previous state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/choice-state.fnl"} }, "choice state properties passed\n");
}

test "editor transitions preserve draft and event ownership" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/editor-state.fnl"} }, "editor state properties passed\n");
}

test "modal editing preserves state and undo boundaries" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/editing-state.fnl"} }, "editing state properties passed\n");
}

test "initializers preserve transaction ownership" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/initialization-state.fnl"} }, "initialization transaction ownership passed\n");
}

test "command invocation uses explicit queued execution" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/command-state.fnl"} }, "explicit command invocation ownership passed\n");
}

test "event routing preserves transaction ownership" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/routing-state.fnl"} }, "routing transaction ownership passed\n");
}

test "dialogs preserve state and protected input ownership" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/dialog-state.fnl"} }, "dialog state properties passed\n");
}

test "authentication preserves startup and dialog state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/auth-state.fnl"} }, "auth state contracts passed\n");
}

test "fake provider preserves state and response fixtures" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/fake-provider-state.fnl"} }, "fake provider state contracts passed\n");
}

test "Codex stream state is immutable and independent of record batching" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/codex-stream-state.fnl"} }, "Codex stream state properties passed\n");
}

test "OpenAI-compatible streams preserve state and batching semantics" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/openai-stream-state.fnl"} }, "OpenAI stream state properties passed\n");
}

test "Anthropic model discovery preserves state across pagination" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/anthropic-discovery-state.fnl"} }, "Anthropic discovery state properties passed\n");
}

test "Anthropic streaming preserves signed state and batching semantics" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/anthropic-stream-state.fnl"} }, "Anthropic stream state properties passed\n");
}

test "Claude records preserve state and deduplicate streamed and final tools" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/claude-stream-state.fnl"} }, "Claude stream state properties passed\n");
}

test "history transitions preserve previous state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/history-state.fnl"} }, "history state contracts passed\n");
}

test "runtime state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/runtime-state.fnl"} }, "runtime state regressions passed\n");
}

test "editor queue" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/editor-queue.fnl"} }, "editor queue regressions passed\n");
}

test "ui layout" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/ui-layout.fnl"} }, "layout contracts passed\n");
}

test "file tool contracts expose hashline reads and compatible edits" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/file-tools.fnl"} }, "file tool contracts passed\n");
}

test "tool summary role keeps background requests out of canonical conversation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/tool-summary.fnl"} }, "tool summary lifecycle passed\n");
}

test "Codex discovers the authenticated model catalogue" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/codex-models.fnl"} }, "Codex model discovery contracts passed\n");
}

test "transcript renders numbered code diffs and useful tool previews" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/transcript-render-details.fnl"} }, "transcript rendering details passed\n");
}

test "selection follows rendered wrapped rows and source ranges" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/selection-rendered-geometry.fnl"} }, "rendered selection geometry passed\n");
}

test "projection regions isolate editor and transcript invalidation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.expect(.{ .binary = options.source_root ++ "/tools/fennel", .cwd = options.source_root, .args = &.{"tests/projection-regions.fnl"} }, "projection region ownership passed\n");
}
