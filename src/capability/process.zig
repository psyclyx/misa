//! Direct-argv process execution and captured-output normalization.
const std = @import("std");

pub const StdoutFormat = enum { text, json_lines, json_lines_stream };

pub const Spec = struct {
    argv: []const std.json.Value,
    completion: []const u8,
    id: []const u8,
    stdout_format: StdoutFormat,
    stdin: ?[]const u8,
    stdin_json: ?std.json.Value,
    timeouts: Timeouts = .{},
    /// Supplied by native composition only; parse never accepts environment
    /// variables from extension effects.
    environment: ?*const std.process.Environ.Map = null,

    pub const Timeouts = struct {
        startup_ms: u64 = 10_000,
        idle_ms: u64 = 30_000,
        overall_ms: u64 = 600_000,
    };

    pub fn parse(object: std.json.ObjectMap) !Spec {
        const argv = switch (object.get("argv") orelse return error.InvalidEffect) {
            .array => |array| array.items,
            else => return error.InvalidEffect,
        };
        if (argv.len == 0) return error.InvalidEffect;
        for (argv, 0..) |arg, index| switch (arg) {
            .string => |string| if ((index == 0 and string.len == 0) or std.mem.indexOfScalar(u8, string, 0) != null) return error.InvalidEffect,
            else => return error.InvalidEffect,
        };
        const format = if (object.get("stdout_format")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else "text";
        const stdin = if (object.get("stdin")) |value| switch (value) {
            .string => |string| if (string.len <= 1024 * 1024) string else return error.EffectTooLarge,
            else => return error.InvalidEffect,
        } else null;
        const stdin_json = object.get("stdin_json");
        if (stdin != null and stdin_json != null) return error.InvalidEffect;
        const timeouts: Timeouts = if (object.get("timeouts")) |value| blk: {
            if (value != .object) return error.InvalidEffect;
            break :blk .{
                .startup_ms = try timeoutField(value.object, "startup_ms", 10_000),
                .idle_ms = try timeoutField(value.object, "idle_ms", 30_000),
                .overall_ms = try timeoutField(value.object, "overall_ms", 600_000),
            };
        } else .{};
        return .{
            .argv = argv,
            .completion = nonEmptyString(object, "completion") orelse return error.InvalidEffect,
            .id = nonEmptyString(object, "id") orelse return error.InvalidEffect,
            .stdout_format = if (std.mem.eql(u8, format, "text")) .text else if (std.mem.eql(u8, format, "json_lines")) .json_lines else if (std.mem.eql(u8, format, "json_lines_stream")) .json_lines_stream else return error.InvalidEffect,
            .stdin = stdin,
            .stdin_json = stdin_json,
            .timeouts = timeouts,
        };
    }
};

pub const StreamSink = struct {
    context: *anyopaque,
    emit: *const fn (context: *anyopaque, json_line: []const u8) anyerror!void,
    activity: ?*const fn (context: *anyopaque) void = null,
};

pub const ActivitySink = struct {
    context: *anyopaque,
    note: *const fn (context: *anyopaque) void,
};

pub const StreamResult = struct { status: i64, stderr: []u8 };

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
    return runWithActivity(allocator, io, spec, null);
}

pub fn runWithActivity(allocator: std.mem.Allocator, io: std.Io, spec: Spec, activity: ?ActivitySink) !Result {
    var argv: std.ArrayList([]const u8) = .empty;
    defer argv.deinit(allocator);
    for (spec.argv) |arg| try argv.append(allocator, arg.string);

    var encoded_stdin: ?[]u8 = null;
    defer if (encoded_stdin) |value| allocator.free(value);
    if (spec.stdin_json) |value| {
        const json = try std.json.Stringify.valueAlloc(allocator, value, .{});
        defer allocator.free(json);
        encoded_stdin = try std.fmt.allocPrint(allocator, "{s}\n", .{json});
    }
    const process_stdin: ?[]const u8 = if (spec.stdin) |value| value else if (encoded_stdin) |value| value else null;
    const captured = capture(allocator, io, argv.items, process_stdin, activity, spec.environment) catch |err| {
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

/// Emit complete JSONL records as stdout arrives. The child and both pipes are
/// owned here so cancellation of the surrounding I/O task kills and reaps it.
pub fn runJsonLines(allocator: std.mem.Allocator, io: std.Io, spec: Spec, sink: StreamSink) !StreamResult {
    var argv: std.ArrayList([]const u8) = .empty;
    defer argv.deinit(allocator);
    for (spec.argv) |arg| try argv.append(allocator, arg.string);
    var encoded: ?[]u8 = null;
    defer if (encoded) |value| allocator.free(value);
    if (spec.stdin_json) |value| {
        const json = try std.json.Stringify.valueAlloc(allocator, value, .{});
        defer allocator.free(json);
        encoded = try std.fmt.allocPrint(allocator, "{s}\n", .{json});
    }
    const stdin = spec.stdin orelse encoded;
    var child = try std.process.spawn(io, .{ .argv = argv.items, .environ_map = spec.environment, .stdin = if (stdin != null) .pipe else .ignore, .stdout = .pipe, .stderr = .pipe });
    defer child.kill(io);
    var stdin_group: std.Io.Group = .init;
    defer stdin_group.cancel(io);
    var pump: WritePump = .{};
    if (stdin) |bytes| {
        const pipe = child.stdin.?;
        child.stdin = null;
        stdin_group.async(io, WritePump.run, .{ &pump, io, pipe, bytes });
    }
    var buffer: std.Io.File.MultiReader.Buffer(2) = undefined;
    var reader: std.Io.File.MultiReader = undefined;
    reader.init(allocator, io, buffer.toStreams(), &.{ child.stdout.?, child.stderr.? });
    defer reader.deinit();
    var line: std.ArrayList(u8) = .empty;
    defer line.deinit(allocator);
    var stderr: std.ArrayList(u8) = .empty;
    errdefer stderr.deinit(allocator);
    while (reader.fill(1, .none)) |_| {
        const stdout_bytes = reader.reader(0).buffered();
        if (stdout_bytes.len != 0) if (sink.activity) |note| note(sink.context);
        try feedJsonLines(allocator, &line, stdout_bytes, sink);
        reader.reader(0).tossBuffered();
        const stderr_bytes = reader.reader(1).buffered();
        if (stderr_bytes.len != 0) if (sink.activity) |note| note(sink.context);
        if (stderr.items.len + stderr_bytes.len > 1024 * 1024) return error.StreamTooLong;
        try stderr.appendSlice(allocator, stderr_bytes);
        reader.reader(1).tossBuffered();
    } else |err| switch (err) {
        error.EndOfStream => {},
        else => |other| return other,
    }
    try reader.checkAnyError();
    try stdin_group.await(io);
    if (pump.failure) |failure| return failure;
    const tail = std.mem.trim(u8, line.items, " \t\r");
    if (tail.len != 0) try emitValidated(allocator, sink, tail);
    const term = try child.wait(io);
    const clean_stderr = try sanitizeOutput(allocator, stderr.items, 1024 * 1024);
    stderr.deinit(allocator);
    return .{ .status = termStatus(term), .stderr = clean_stderr };
}

fn feedJsonLines(allocator: std.mem.Allocator, line: *std.ArrayList(u8), bytes: []const u8, sink: StreamSink) !void {
    for (bytes) |byte| if (byte == '\n') {
        const value = std.mem.trim(u8, line.items, " \t\r");
        if (value.len != 0) try emitValidated(allocator, sink, value);
        line.clearRetainingCapacity();
    } else {
        try line.append(allocator, byte);
        if (line.items.len > 256 * 1024) return error.StreamRecordTooLarge;
    };
}

fn emitValidated(allocator: std.mem.Allocator, sink: StreamSink, line: []const u8) !void {
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, line, .{});
    defer parsed.deinit();
    try sink.emit(sink.context, line);
}

fn termStatus(term: std.process.Child.Term) i64 {
    return switch (term) {
        .exited => |code| code,
        .signal => |signal| -@as(i64, @intCast(@intFromEnum(signal))),
        .stopped => |signal| -@as(i64, @intCast(@intFromEnum(signal))),
        .unknown => |code| @intCast(code),
    };
}

fn capture(allocator: std.mem.Allocator, io: std.Io, argv: []const []const u8, stdin: ?[]const u8, activity: ?ActivitySink, environ: ?*const std.process.Environ.Map) !std.process.RunResult {
    var child = try std.process.spawn(io, .{ .argv = argv, .environ_map = environ, .stdin = if (stdin != null) .pipe else .ignore, .stdout = .pipe, .stderr = .pipe });
    defer child.kill(io);
    var stdin_group: std.Io.Group = .init;
    defer stdin_group.cancel(io);
    var pump: WritePump = .{};
    if (stdin) |input| {
        const pipe = child.stdin.?;
        child.stdin = null;
        stdin_group.async(io, WritePump.run, .{ &pump, io, pipe, input });
    }

    var buffer: std.Io.File.MultiReader.Buffer(2) = undefined;
    var reader: std.Io.File.MultiReader = undefined;
    reader.init(allocator, io, buffer.toStreams(), &.{ child.stdout.?, child.stderr.? });
    defer reader.deinit();
    var stdout_seen: usize = 0;
    var stderr_seen: usize = 0;
    while (reader.fill(64, .none)) |_| {
        const stdout_len = reader.reader(0).buffered().len;
        const stderr_len = reader.reader(1).buffered().len;
        if (stdout_len > stdout_seen or stderr_len > stderr_seen) if (activity) |sink| sink.note(sink.context);
        stdout_seen = stdout_len;
        stderr_seen = stderr_len;
        if (stdout_len > 1024 * 1024 or stderr_len > 1024 * 1024) return error.StreamTooLong;
    } else |err| switch (err) {
        error.EndOfStream => {},
        else => |other| return other,
    }
    try reader.checkAnyError();
    try stdin_group.await(io);
    if (pump.failure) |failure| return failure;
    const term = try child.wait(io);
    const stdout = try reader.toOwnedSlice(0);
    errdefer allocator.free(stdout);
    return .{ .term = term, .stdout = stdout, .stderr = try reader.toOwnedSlice(1) };
}

const WritePump = struct {
    failure: ?anyerror = null,

    fn run(self: *WritePump, io: std.Io, pipe: std.Io.File, bytes: []const u8) std.Io.Cancelable!void {
        defer pipe.close(io);
        pipe.writeStreamingAll(io, bytes) catch |err| {
            if (err == error.Canceled) return error.Canceled;
            self.failure = err;
        };
    }
};

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

fn timeoutField(object: std.json.ObjectMap, name: []const u8, default: u64) !u64 {
    const value = object.get(name) orelse return default;
    if (value != .integer or value.integer < 1 or value.integer > 3_600_000) return error.InvalidEffect;
    return @intCast(value.integer);
}

fn optionalString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |item| item,
        else => null,
    };
}

fn nonEmptyString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const string = optionalString(object, name) orelse return null;
    return if (string.len != 0 and std.mem.indexOfScalar(u8, string, 0) == null) string else null;
}

test "JSONL callback runs before delayed producer completion" {
    const Probe = struct {
        io: std.Io,
        first: ?std.Io.Timestamp = null,
        count: usize = 0,
        fn emit(context: *anyopaque, line: []const u8) anyerror!void {
            const self: *@This() = @ptrCast(@alignCast(context));
            if (self.first == null) self.first = .now(self.io, .awake);
            self.count += 1;
            _ = line;
        }
    };
    var argv = [_]std.json.Value{
        .{ .string = "/bin/sh" },
        .{ .string = "-c" },
        .{ .string = "printf '{\"n\":1}\\n'; sleep 0.25; printf '{\"n\":2}\\n'" },
    };
    var probe: Probe = .{ .io = std.testing.io };
    const result = try runJsonLines(std.testing.allocator, std.testing.io, .{
        .argv = &argv,
        .completion = "done",
        .id = "test",
        .stdout_format = .json_lines_stream,
        .stdin = null,
        .stdin_json = null,
    }, .{ .context = &probe, .emit = Probe.emit });
    defer std.testing.allocator.free(result.stderr);
    const finished: std.Io.Timestamp = .now(std.testing.io, .awake);
    try std.testing.expectEqual(@as(usize, 2), probe.count);
    try std.testing.expect(finished.nanoseconds - probe.first.?.nanoseconds >= 100 * std.time.ns_per_ms);
}

test "stdin and stdout are pumped concurrently" {
    const input = try std.testing.allocator.alloc(u8, 256 * 1024);
    defer std.testing.allocator.free(input);
    @memset(input, 'x');
    const argv = [_][]const u8{
        "/bin/sh",
        "-c",
        // Both directions exceed ordinary pipe capacity. Sequential pumping
        // deadlocks because the child writes before it reads.
        "dd if=/dev/zero bs=262144 count=1 2>/dev/null; cat >/dev/null",
    };
    const captured = try capture(std.testing.allocator, std.testing.io, &argv, input, null, null);
    defer std.testing.allocator.free(captured.stdout);
    defer std.testing.allocator.free(captured.stderr);
    try std.testing.expectEqual(@as(i64, 0), termStatus(captured.term));
    try std.testing.expectEqual(@as(usize, 256 * 1024), captured.stdout.len);
}

test "output sanitizer removes terminal controls and repairs utf8" {
    const clean = try sanitizeOutput(std.testing.allocator, "ok\tred\x1b[31m!\x1b[0m\r\n\x00\xc2\x85\xff!", 1024);
    defer std.testing.allocator.free(clean);
    try std.testing.expectEqualStrings("ok red!\n�!", clean);
    try std.testing.expect(std.unicode.utf8ValidateSlice(clean));
    try std.testing.expect(std.mem.indexOfScalar(u8, clean, 0x1b) == null);
}
