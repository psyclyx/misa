const std = @import("std");
const support = @import("harness.zig");
const Harness = support.Harness;

fn expectFailure(h: *Harness, invocation: Harness.Invocation, needles: []const []const u8) !void {
    const result = try h.run(invocation);
    if (result.term != .exited or result.term.exited == 0) {
        std.debug.print("expected failure containing {s}, got {any}\nstdout:\n{s}\nstderr:\n{s}\n", .{ needles[0], result.term, result.stdout, result.stderr });
        return error.ExpectedFailure;
    }
    for (needles) |needle| try support.contains(result.stderr, needle);
}

test "invalid extension and callback contracts report actionable errors" {
    inline for (.{
        .{ "bad", "misa.providers.unknown" },
        .{ "fail", "exploded" },
        .{ "missing-definitions", "configuration must contain a definitions table" },
        .{ "malformed-module", "configuration must contain a definitions table" },
        .{ "constructor-trace", "constructor exploded" },
        .{ "late-effect", "UnknownNativeEffect" },
        .{ "nul-extension", "No such file or directory" },
        .{ "nul-command", "command argv must contain nonempty strings without NUL" },
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
        \\(let [config {:models {:default 42}}
        \\      app ((require :tests.application) {:config config})]
        \\  (app.include (. (require :tests.stock) :misa.models))
        \\  {:config config :definitions app.definitions})
    );
    try expectFailure(&h, .{ .binary = @import("integration_options").installed_binary }, &.{ "models/init.lua:", "config.models.default must be a nonempty string", "stack traceback:" });
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
