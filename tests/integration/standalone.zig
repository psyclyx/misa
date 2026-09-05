const Harness = @import("harness.zig").Harness;
const options = @import("integration_options");

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
