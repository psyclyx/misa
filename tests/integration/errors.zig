const std = @import("std");
const support = @import("harness.zig");
const Harness = support.Harness;

fn expectFailure(h: *Harness, invocation: Harness.Invocation, needles: []const []const u8) !void {
    const result = try h.run(invocation);
    try std.testing.expect(result.term == .exited and result.term.exited != 0);
    for (needles) |needle| try support.contains(result.stderr, needle);
}

test "invalid extension and callback contracts report actionable errors" {
    inline for (.{
        .{ "bad", "misa.providers.unknown" },
        .{ "fail", "exploded" },
        .{ "missing-definitions", "module requires definitions" },
        .{ "malformed-module", "module returned no definitions" },
        .{ "constructor-trace", "constructor exploded" },
        .{ "late-effect", "UnknownNativeEffect" },
        .{ "nul-extension", "No such file or directory" },
        .{ "nul-command", "command argv must not contain NUL" },
    }) |case| {
        var h = try Harness.init();
        defer h.deinit();
        try h.config(@embedFile("configs/" ++ case[0] ++ ".fnl"));
        try expectFailure(&h, .{}, &.{case[1]});
        if (comptime std.mem.eql(u8, case[0], "fail") or std.mem.eql(u8, case[0], "constructor-trace")) try expectFailure(&h, .{}, &.{"stack traceback:"});
    }
}

test "duplicate config arguments are rejected" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config("{:definitions {}}");
    try expectFailure(&h, .{ .args = &.{ "--config", "config.fnl", "--config", "config.fnl" } }, &.{"--config may only be specified once"});
}

test "installed generated extensions preserve actionable diagnostics" {
    var h = try Harness.init();
    defer h.deinit();
    _ = h.environ.swapRemove("MISA_EXTENSION_DIR");
    try h.config(
        \\(local standard (require :misa.standard))
        \\(standard.application {:config {:models {:default 42}}
        \\                       :modules {:models {:build (require :misa.agent.models)}}})
    );
    try expectFailure(&h, .{ .binary = @import("integration_options").installed_binary }, &.{ "models.lua:", "config.models.default must be a nonempty string", "stack traceback:" });
}

test "configuration conversion bounds nesting" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(
        \\(var deep {})
        \\(for [i 1 130] (set deep {:x deep}))
        \\{:config deep :definitions {}}
    );
    const result = try h.run(.{});
    try std.testing.expect(result.term == .exited and result.term.exited != 0);
    try std.testing.expect(std.mem.indexOf(u8, result.stderr, "nesting") != null or std.mem.indexOf(u8, result.stderr, "depth") != null);
}
