const std = @import("std");
const support = @import("harness.zig");
const Harness = support.Harness;

test "state saves privately and reloads in a fresh process" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/state.json"));
    try h.expect(.{}, "");
    const stat = try h.temporary.dir.statFile(support.io, "application-state.json", .{});
    try std.testing.expectEqual(@as(u32, 0o600), stat.permissions.toMode() & 0o777);
    try h.expect(.{}, "state loaded\n");
    try support.contains(try h.read("application-state.json"), "\"integration\"");
}

test "invalid persistence reaches the asynchronous completion event" {
    var h = try Harness.init();
    defer h.deinit();
    try h.write("application-state.json", @embedFile("configs/invalid-state.json"));
    try h.config(@embedFile("configs/invalid-state-config.json"));
    try h.expect(.{}, "");
}

test "API key login and logout use a private XDG credential store" {
    var h = try Harness.init();
    defer h.deinit();
    _ = h.environ.swapRemove("MISA_AUTH_FILE");
    const login = try h.run(.{ .args = &.{ "login", "openai" }, .input = "test-secret\n" });
    try support.success(login);
    try support.contains(login.stderr, "saved openai credential");
    try std.testing.expect(std.mem.indexOf(u8, login.stderr, "test-secret") == null);
    const directory = try h.temporary.dir.statFile(support.io, "misa", .{});
    const file = try h.temporary.dir.statFile(support.io, "misa/auth.json", .{});
    try std.testing.expectEqual(@as(u32, 0o700), directory.permissions.toMode() & 0o777);
    try std.testing.expectEqual(@as(u32, 0o600), file.permissions.toMode() & 0o777);
    try support.contains(try h.read("misa/auth.json"), "test-secret");
    try h.expect(.{ .args = &.{ "status", "openai" } }, "logged in\n");
    try h.expect(.{ .args = &.{ "logout", "openai" } }, "");
    try h.expect(.{ .args = &.{ "status", "openai" } }, "logged out\n");
    try std.testing.expect(std.mem.indexOf(u8, try h.read("misa/auth.json"), "test-secret") == null);
}

test "Claude authentication delegates to the CLI without copying credentials" {
    var h = try Harness.init();
    defer h.deinit();
    try h.executable("claude",
        \\#!/bin/sh
        \\[ "$1" = auth ]
        \\case "$2" in login|logout) ;; status) printf '{"loggedIn":true,"subscriptionType":"max"}\n' ;; *) exit 1 ;; esac
    );
    try h.claudeFixture("claude");
    try h.expect(.{ .args = &.{ "login", "claude" } }, "");
    try h.expect(.{ .args = &.{ "status", "claude" } }, "logged in (max)\n");
    try h.expect(.{ .args = &.{ "logout", "claude" } }, "");
    try std.testing.expectError(error.FileNotFound, h.temporary.dir.statFile(support.io, "auth.json", .{}));
}

test "favoriting a command choice persists its state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-command-choice.json"));
    try h.expect(.{ .input = "/choose\ntwo beta\x1bv\n" }, "beta-two\n");
    try support.contains(try h.read("application-state.json"), "\"favorite\": true");
}
