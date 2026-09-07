const Harness = @import("harness.zig").Harness;
const options = @import("integration_options");

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
