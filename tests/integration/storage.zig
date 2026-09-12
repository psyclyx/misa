const std = @import("std");
const support = @import("harness.zig");
const Harness = support.Harness;

test "state saves privately and reloads in a fresh process" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/state.fnl"));
    try h.expect(.{}, "");
    const stat = try h.temporary.dir.statFile(support.io, "application-state.json", .{});
    try std.testing.expectEqual(@as(u32, 0o600), stat.permissions.toMode() & 0o777);
    try h.expect(.{}, "state loaded\n");
    try support.contains(try h.read("application-state.json"), "\"integration\"");
}

test "conversation appends persist privately and continue in a fresh process" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/conversation.fnl"));
    try h.expect(.{}, "appended 2 at 2\n");

    // The database lives under XDG_STATE_HOME in a directory Misa creates for
    // itself, and the sequence number continues where the first process left
    // off rather than restarting.
    const directory = try h.temporary.dir.statFile(support.io, "misa", .{});
    try std.testing.expectEqual(@as(u32, 0o700), directory.permissions.toMode() & 0o777);
    const database = try h.temporary.dir.statFile(support.io, "misa/conversations.sqlite3", .{});
    try std.testing.expect(database.size > 0);
    try h.expect(.{}, "appended 2 at 4\n");
}

test "a stored conversation lists and reopens with its label" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/conversation-read.fnl"));
    try h.expect(.{}, "read 2 entries\n");
}

test "invalid persistence reaches the asynchronous completion event" {
    var h = try Harness.init();
    defer h.deinit();
    try h.write("application-state.json", @embedFile("configs/invalid-state.json"));
    try h.config(@embedFile("configs/invalid-state-config.fnl"));
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

test "named accounts coexist, switch, and are removed one at a time" {
    var h = try Harness.init();
    defer h.deinit();
    _ = h.environ.swapRemove("MISA_AUTH_FILE");
    const first = try h.run(.{ .args = &.{ "login", "openai", "work" }, .input = "work-secret\n" });
    try support.success(first);
    try support.contains(first.stderr, "account work");
    try std.testing.expect(std.mem.indexOf(u8, first.stderr, "work-secret") == null);
    // A single account keeps its name in status, because it is not the default.
    try h.expect(.{ .args = &.{ "status", "openai" } }, "work: logged in\n");

    // A later login adds an account and becomes the one requests use.
    const second = try h.run(.{ .args = &.{ "login", "openai", "personal" }, .input = "personal-secret\n" });
    try support.success(second);
    try h.expect(.{ .args = &.{ "status", "openai" } }, "personal: logged in (active)\nwork: logged in\n");
    try h.expect(.{ .args = &.{ "status", "openai", "work" } }, "work: logged in\n");
    try support.contains(try h.read("misa/auth.json"), "work-secret");
    try support.contains(try h.read("misa/auth.json"), "personal-secret");

    // Selecting switches the stored account without authenticating again.
    try h.expect(.{ .args = &.{ "select", "openai", "work" } }, "using account work\n");
    try h.expect(.{ .args = &.{ "status", "openai" } }, "personal: logged in\nwork: logged in (active)\n");

    // Removing one account leaves the other, and the file keeps no secrets of it.
    try h.expect(.{ .args = &.{ "logout", "openai", "work" } }, "");
    try h.expect(.{ .args = &.{ "status", "openai" } }, "personal: logged in\n");
    try std.testing.expect(std.mem.indexOf(u8, try h.read("misa/auth.json"), "work-secret") == null);

    // Usage problems and unknown names fail before anything is written.
    const usage = try h.run(.{ .args = &.{ "select", "openai" } });
    try std.testing.expectEqual(@as(u8, 2), usage.term.exited);
    try support.contains(usage.stderr, "usage: misa select");
    const absent = try h.run(.{ .args = &.{ "select", "openai", "absent" } });
    try std.testing.expectEqual(@as(u8, 2), absent.term.exited);
    try support.contains(absent.stderr, "unknown account");
    const invalid = try h.run(.{ .args = &.{ "login", "openai", "two words" } });
    try std.testing.expectEqual(@as(u8, 2), invalid.term.exited);
    try support.contains(invalid.stderr, "invalid account name");

    // Logging out without an account removes every account of the provider.
    try h.expect(.{ .args = &.{ "logout", "openai" } }, "");
    try h.expect(.{ .args = &.{ "status", "openai" } }, "logged out\n");
    try std.testing.expect(std.mem.indexOf(u8, try h.read("misa/auth.json"), "personal-secret") == null);
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
    // Claude Code owns its one account, so Misa names none for it.
    const rejected = try h.run(.{ .args = &.{ "select", "claude", "work" } });
    try std.testing.expectEqual(@as(u8, 2), rejected.term.exited);
    try support.contains(rejected.stderr, "unknown account");
    const named = try h.run(.{ .args = &.{ "status", "claude", "work" } });
    try std.testing.expectEqual(@as(u8, 2), named.term.exited);
    try support.contains(named.stderr, "unknown account");
    try h.expect(.{ .args = &.{ "logout", "claude" } }, "");
    try std.testing.expectError(error.FileNotFound, h.temporary.dir.statFile(support.io, "auth.json", .{}));
}

test "favoriting a command choice persists its state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/generic-command-choice.fnl"));
    try h.expect(.{ .input = "/choose\ntwo beta\x1bv\n" }, "beta-two\n");
    try support.contains(try h.read("application-state.json"), "\"favorite\": true");
}
