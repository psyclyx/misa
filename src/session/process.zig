//! Direct-argv process execution and captured-output normalization.
const std = @import("std");

pub const Spec = struct {
    argv: []const std.json.Value,
    completion: []const u8,
    id: []const u8,

    pub fn parse(object: std.json.ObjectMap) !Spec {
        if (object.get("stdin") != null) return error.UnsupportedProcessStdin;
        const argv = switch (object.get("argv") orelse return error.InvalidEffect) {
            .array => |array| array.items,
            else => return error.InvalidEffect,
        };
        if (argv.len == 0) return error.InvalidEffect;
        for (argv) |arg| switch (arg) {
            .string => |string| if (string.len == 0 or std.mem.indexOfScalar(u8, string, 0) != null) return error.InvalidEffect,
            else => return error.InvalidEffect,
        };
        return .{
            .argv = argv,
            .completion = nonEmptyString(object, "completion") orelse return error.InvalidEffect,
            .id = nonEmptyString(object, "id") orelse return error.InvalidEffect,
        };
    }
};

pub const Result = struct {
    status: i64,
    stdout: []u8,
    stderr: []u8,

    pub fn deinit(self: Result, allocator: std.mem.Allocator) void {
        allocator.free(self.stdout);
        allocator.free(self.stderr);
    }
};

pub fn run(allocator: std.mem.Allocator, io: std.Io, spec: Spec) !Result {
    var argv: std.ArrayList([]const u8) = .empty;
    defer argv.deinit(allocator);
    for (spec.argv) |arg| try argv.append(allocator, arg.string);

    const captured = std.process.run(allocator, io, .{
        .argv = argv.items,
        .stdout_limit = .limited(1024 * 1024),
        .stderr_limit = .limited(1024 * 1024),
    }) catch |err| {
        const stdout = try allocator.dupe(u8, "");
        errdefer allocator.free(stdout);
        return .{
            .status = -1,
            .stdout = stdout,
            .stderr = try allocator.dupe(u8, @errorName(err)),
        };
    };
    defer allocator.free(captured.stdout);
    defer allocator.free(captured.stderr);
    const stdout = try sanitizeOutput(allocator, captured.stdout, 1024 * 1024);
    errdefer allocator.free(stdout);
    return .{
        .status = switch (captured.term) {
            .exited => |code| code,
            .signal => |signal| -@as(i64, @intCast(@intFromEnum(signal))),
            .stopped => |signal| -@as(i64, @intCast(@intFromEnum(signal))),
            .unknown => |code| @intCast(code),
        },
        .stdout = stdout,
        .stderr = try sanitizeOutput(allocator, captured.stderr, 1024 * 1024),
    };
}

/// Make captured bytes safe for JSON and later semantic presentation.
pub fn sanitizeOutput(allocator: std.mem.Allocator, input: []const u8, limit: usize) ![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    var i: usize = 0;
    while (i < input.len and out.items.len < limit) {
        const byte = input[i];
        if (byte == 0x1b) {
            i += 1;
            if (i < input.len and input[i] == '[') {
                i += 1;
                while (i < input.len) : (i += 1) if (input[i] >= 0x40 and input[i] <= 0x7e) {
                    i += 1;
                    break;
                };
            } else if (i < input.len and input[i] == ']') {
                i += 1;
                while (i < input.len) {
                    if (input[i] == 7) {
                        i += 1;
                        break;
                    }
                    if (input[i] == 0x1b and i + 1 < input.len and input[i + 1] == '\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else if (i < input.len) i += 1;
            continue;
        }
        if (byte == '\r') {
            try out.append(allocator, '\n');
            i += 1;
            if (i < input.len and input[i] == '\n') i += 1;
            continue;
        }
        if (byte == '\n') {
            try out.append(allocator, byte);
            i += 1;
            continue;
        }
        if (byte == '\t') {
            try out.append(allocator, ' ');
            i += 1;
            continue;
        }
        if (byte < 0x20 or byte == 0x7f) {
            i += 1;
            continue;
        }
        const len = std.unicode.utf8ByteSequenceLength(byte) catch {
            if (out.items.len + 3 <= limit) try out.appendSlice(allocator, "\xef\xbf\xbd");
            i += 1;
            continue;
        };
        if (i + len > input.len) {
            if (out.items.len + 3 <= limit) try out.appendSlice(allocator, "\xef\xbf\xbd");
            break;
        }
        const cp = std.unicode.utf8Decode(input[i .. i + len]) catch {
            if (out.items.len + 3 <= limit) try out.appendSlice(allocator, "\xef\xbf\xbd");
            i += 1;
            continue;
        };
        if (cp >= 0x80 and cp <= 0x9f) {
            i += len;
            continue;
        }
        if (out.items.len + len > limit) break;
        try out.appendSlice(allocator, input[i .. i + len]);
        i += len;
    }
    return out.toOwnedSlice(allocator);
}

fn nonEmptyString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    const string = switch (value) {
        .string => |item| item,
        else => return null,
    };
    return if (string.len != 0 and std.mem.indexOfScalar(u8, string, 0) == null) string else null;
}

test "output sanitizer removes terminal controls and repairs utf8" {
    const clean = try sanitizeOutput(std.testing.allocator, "ok\tred\x1b[31m!\x1b[0m\r\n\x00\xc2\x85\xff!", 1024);
    defer std.testing.allocator.free(clean);
    try std.testing.expectEqualStrings("ok red!\n�!", clean);
    try std.testing.expect(std.unicode.utf8ValidateSlice(clean));
    try std.testing.expect(std.mem.indexOfScalar(u8, clean, 0x1b) == null);
}
