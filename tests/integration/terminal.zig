const Harness = @import("harness.zig").Harness;
const options = @import("integration_options");

test "terminal coeffects preserve geometry and plain output" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/context.fnl"));
    try h.environ.put("COLUMNS", "37");
    try h.environ.put("LINES", "11");
    try h.expect(.{ .args = &.{"arg"} }, "plain\n");
}

test "installed profile resolves without source overrides" {
    var h = try Harness.init();
    defer h.deinit();
    _ = h.environ.swapRemove("MISA_CONFIG");
    _ = h.environ.swapRemove("MISA_EXTENSION_DIR");
    try h.expect(.{ .binary = options.installed_binary }, "misa> enter a prompt:\n");
}

test "installed binary supports explicit source catalog overrides" {
    var h = try Harness.init();
    defer h.deinit();
    try h.environ.put("MISA_EXTENSION_DIR", options.source_root ++ "/extensions");
    try h.expect(.{ .binary = options.installed_binary }, "misa> enter a prompt:\n");
}

test "editor insertion deletion and movement preserve grapheme boundaries" {
    const cases = [_][2][]const u8{
        .{ "ac\x1b[Db\x1b[D\x1b[Cd\n", "abdc\n" },
        .{ "aéx\x1b[D\x7f\n", "ax\n" },
        .{ "aéx\x1b[D\x7f\n", "ax\n" },
        .{ "a👩‍💻x\x1b[D\x7f\n", "ax\n" },
        .{ "aक्x\x1b[D\x7f\n", "ax\n" },
        .{ "aक्षx\x1b[D\x7f\n", "ax\n" },
        .{ "क्षx\x1b[D\x1b[D\x1b[CZ\n", "क्षZx\n" },
        .{ "éx\x1b[D\x1b[D\x1b[CZ\n", "éZx\n" },
        .{ "\x1b[200~ab\ncd\x1b[201~\x1b[D\x7fX\n", "ab\nXd\n" },
    };
    for (cases) |case| {
        var h = try Harness.init();
        defer h.deinit();
        try h.executable("echo-prompt", @embedFile("fixtures/integration-echo-prompt"));
        try h.config(@embedFile("configs/editor.fnl"));
        try h.expect(.{ .input = case[0] }, case[1]);
    }
}

fn preparePty(h: *Harness) !void {
    if (@import("builtin").os.tag != .linux) return error.SkipZigTest;
    const probe = h.run(.{ .binary = "script", .args = &.{"--version"} }) catch |err| switch (err) {
        error.FileNotFound => return error.SkipZigTest,
        else => return err,
    };
    if (@import("std").mem.indexOf(u8, probe.stdout, "util-linux") == null) return error.SkipZigTest;
    try h.environ.put("TERM", "xterm");
    try h.executable("pty-runner",
        \\#!/bin/sh
        \\stty rows 12 cols 40 </dev/tty
        \\(sleep 0.20; stty rows 20 cols 60 </dev/tty; kill -WINCH "$$") &
        \\exec "@BIN@"
    );
}

fn expectOrdered(output: []const u8, needles: []const []const u8) !void {
    var remaining = output;
    for (needles) |needle| {
        try @import("harness.zig").contains(remaining, needle);
        remaining = remaining[@import("std").mem.indexOf(u8, remaining, needle).? + needle.len ..];
    }
}

test "PTY redraw resize and alternate screen lifecycle" {
    var h = try Harness.init();
    defer h.deinit();
    try preparePty(&h);
    try h.config(@embedFile("configs/pty-redraw.fnl"));
    const result = try h.run(.{ .binary = "script", .args = &.{ "-qefc", "./pty-runner", "/dev/null" } });
    try @import("harness.zig").success(result);
    try expectOrdered(result.stdout, &.{ "\x1b[?1049h", "partial transcript", "resized 60x20", "producer completed", "\x1b[?1049l" });
}

test "PTY Ctrl C cancels the child and restores the screen" {
    const std = @import("std");
    const support = @import("harness.zig");
    var h = try Harness.init();
    defer h.deinit();
    try preparePty(&h);
    try h.config(@embedFile("configs/pty-cancel.fnl"));
    const result = try h.run(.{ .binary = "script", .args = &.{ "-qefc", "./pty-runner", "/dev/null" }, .input = "\x03", .input_delay_ms = 300 });
    try support.success(result);
    try expectOrdered(result.stdout, &.{ "\x1b[?1049h", "partial transcript", "cancelled child", "\x1b[?1049l" });
    const pid = std.mem.trim(u8, try h.read("child.pid"), "\r\n ");
    _ = try std.fmt.parseInt(u32, pid, 10);
    for (0..20) |_| {
        const alive = try h.run(.{ .binary = "kill", .args = &.{ "-0", pid } });
        if (alive.term == .exited and alive.term.exited != 0) return;
        try std.Io.sleep(support.io, .fromMilliseconds(50), .awake);
    }
    return error.ChildSurvivedCancellation;
}

test "process deadline still fires after the child closes output streams" {
    var h = try Harness.init();
    defer h.deinit();
    try @import("std").testing.expectError(error.Timeout, h.run(.{ .binary = "/bin/sh", .args = &.{ "-c", "exec 1>&- 2>&-; exec sleep 10" }, .timeout_ms = 100 }));
}

test "process deadline cancels a blocked stdin writer" {
    const std = @import("std");
    var h = try Harness.init();
    defer h.deinit();
    const input = try h.allocator().alloc(u8, 1024 * 1024);
    @memset(input, 'x');
    try std.testing.expectError(error.Timeout, h.run(.{ .binary = "sleep", .args = &.{"10"}, .input = input, .timeout_ms = 100 }));
}
