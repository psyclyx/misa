const Harness = @import("harness.zig").Harness;

test "auth ui" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/auth-ui.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/status op\t\n" }, "logged out\n");
}

test "auth ui 2" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/auth-ui.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/status\nopenai\n" }, "logged out\n");
}

test "picker hints" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/picker-hints.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "hints\n");
}

test "choice contracts" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/choice-contracts.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "choice contracts\n");
}

test "command completion" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/command-completion.fnl"));
    try h.expect(.{ .args = &.{}, .input = "discard me\x03/p\t\n" }, "pong\n");
}

test "command completion 2" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/command-completion.fnl"));
    try h.expect(.{ .args = &.{}, .input = "\x04" }, "");
}

test "Enter accepts and invokes a partial command without Tab" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/command-completion.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/p\n" }, "pong\n");
}

test "Enter honors highlighted completion over an exact typed command" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/command-completion.fnl"));
    try h.expect(.{ .input = "/ping\x1b[B\n" }, "pongpong\n");
}

test "inline choices" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/inline-choices.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/\x1bz\n" }, "inline hotkey\n");
}

test "inline choices 2" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/inline-choices.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/choose \x1bxb\n\n" }, "promoted inline\n");
}

test "inline choices 3" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/inline-choices.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/choose \x1b/" }, "inline view picker\n");
}

test "command narrowing" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/command-narrowing.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/mod\x1bj\x1bj" }, "narrowed model\n");
}

test "Enter completes a command then accepts its argument" {
    for ([_][]const u8{ "/mod\n\n", "/model\n\n" }) |input| {
        var h = try Harness.init();
        defer h.deinit();
        try h.config(@embedFile("configs/command-narrowing.fnl"));
        try h.expect(.{ .input = input }, "narrowed model\n");
    }
}

test "command history" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/command-history.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/alpha\n/alpha\n/zulu one\n" }, "command history\n");
}

test "generic picker" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-picker.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/choose\nbeta\n" }, "beta/path\n");
}

test "generic picker 2" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-picker.fnl"));
    try h.environ.put("COLUMNS", "120");
    try h.expect(.{ .args = &.{}, .input = "/panels\n\x1bud" }, "beta/path\n");
}

test "generic picker 3" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-picker.fnl"));
    try h.environ.put("COLUMNS", "40");
    // A prefix with no visible targets does not enter a pending sequence.
    try h.expect(.{ .args = &.{}, .input = "/panels\n\x1bu\n" }, "alpha\n");
}

test "generic command choice" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-command-choice.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/choose\ntwo beta\n" }, "beta-two\n");
}

test "generic command choice 2" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-command-choice.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/choose\ntwo beta\x1bv\n" }, "beta-two\n");
}

test "model picker filter" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/model-picker-filter.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/model\nsecond picker\nhello\n" }, "picked vendor/second\n");
}

test "interaction contracts compose selection dialogs editing and pickers" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/interaction.fnl"));
    try h.expect(.{}, "interaction\n");
}

test "generic picker cancellation returns its correlation" {
    const std = @import("std");
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-picker.fnl"));
    const result = try h.run(.{ .input = "/choose\n\x1b" });
    try @import("harness.zig").success(result);
    const prefix = "cancelled test:";
    try std.testing.expect(std.mem.startsWith(u8, result.stdout, prefix));
    _ = try std.fmt.parseInt(u64, std.mem.trimEnd(u8, result.stdout[prefix.len..], "\n"), 10);
}

test "command choices accept interactive direct and narrowed invocation" {
    for ([_][]const u8{ "/model\nvendor/one\n", "/model vendor/one\n", "/mod\x1bjvendor/one\n" }) |input| {
        var h = try Harness.init();
        defer h.deinit();
        try h.config(@embedFile("configs/command-choice.fnl"));
        try h.expect(.{ .input = input }, "command choices\n");
    }
}
