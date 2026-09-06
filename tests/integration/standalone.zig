const Harness = @import("harness.zig").Harness;
const options = @import("integration_options");

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
