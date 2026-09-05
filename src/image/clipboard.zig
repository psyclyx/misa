//! Raw clipboard image acquisition through direct platform command arguments.
const std = @import("std");
const builtin = @import("builtin");
pub fn read(allocator: std.mem.Allocator, io: std.Io, configured: []const std.json.Value, environ: *const std.process.Environ.Map, max_bytes: usize) ![]u8 {
    if (configured.len != 0) {
        const argv = try allocator.alloc([]const u8, configured.len);
        defer allocator.free(argv);
        for (configured, argv) |arg, *item| item.* = arg.string;
        return capture(allocator, io, argv, max_bytes);
    }
    if (builtin.os.tag == .macos) {
        const hex = try capture(allocator, io, &.{ "osascript", "-e", "the clipboard as «class PNGf»" }, max_bytes * 2 + 256);
        defer allocator.free(hex);
        const marker = std.mem.indexOf(u8, hex, "PNGf") orelse return error.ClipboardHasNoImage;
        const last = std.mem.indexOfPos(u8, hex, marker + 4, "»") orelse return error.ClipboardHasNoImage;
        const value = hex[marker + 4 .. last];
        if (value.len % 2 != 0 or value.len / 2 > max_bytes) return error.ImageTooLarge;
        const bytes = try allocator.alloc(u8, value.len / 2);
        errdefer allocator.free(bytes);
        _ = try std.fmt.hexToBytes(bytes, value);
        return bytes;
    }
    if (environ.get("WAYLAND_DISPLAY") != null) {
        for ([_][]const u8{ "image/png", "image/jpeg" }) |mime| {
            if (capture(allocator, io, &.{ "wl-paste", "--no-newline", "--type", mime }, max_bytes)) |bytes| return bytes else |err| if (err == error.Canceled) return error.Canceled;
        }
    }
    for ([_][]const u8{ "image/png", "image/jpeg" }) |mime| {
        if (capture(allocator, io, &.{ "xclip", "-selection", "clipboard", "-t", mime, "-o" }, max_bytes)) |bytes| return bytes else |err| if (err == error.Canceled) return error.Canceled;
    }
    return error.ClipboardImageUnavailable;
}

fn capture(allocator: std.mem.Allocator, io: std.Io, argv: []const []const u8, limit: usize) ![]u8 {
    const result = try std.process.run(allocator, io, .{ .argv = argv, .stdout_limit = .limited(limit), .stderr_limit = .limited(16 * 1024) });
    defer allocator.free(result.stderr);
    errdefer allocator.free(result.stdout);
    if (result.term != .exited or result.term.exited != 0 or result.stdout.len == 0) return error.ClipboardHasNoImage;
    return result.stdout;
}
