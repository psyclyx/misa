const Harness = @import("harness.zig").Harness;
const options = @import("integration_options");

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
