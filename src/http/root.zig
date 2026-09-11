//! Generic HTTP mechanism. Provider policy supplies protocol data while this
//! module injects credentials without exposing secrets to Lua.
const std = @import("std");
const auth = @import("misa_auth");

pub const Spec = struct {
    url: []const u8,
    method: std.http.Method,
    body: ?[]const u8,
    json: ?std.json.Value,
    headers: []const std.json.Value,
    credential: ?Credential,
    completion: []const u8,
    id: []const u8,
    response_format: enum { text, json, sse_json, sse_json_stream },
    timeouts: Timeouts = .{},
    retries: Retries = .{},

    /// A provider that is momentarily unavailable may be asked again. The
    /// budget is attempts, not time: the operation deadline still bounds the
    /// whole request.
    pub const Retries = struct {
        attempts: u64 = 2,
        backoff_ms: u64 = 500,
        max_ms: u64 = 8_000,
    };

    pub const Timeouts = struct {
        // DNS and connection establishment are part of first-byte latency: the
        // transport does not expose a separate connection-established signal.
        first_byte_ms: u64 = 30_000,
        idle_ms: u64 = 30_000,
        overall_ms: u64 = 600_000,
    };

    pub const Credential = struct {
        id: []const u8,
        header: []const u8,
        prefix: []const u8,
        metadata_field: ?[]const u8,
        metadata_header: ?[]const u8,
    };

    pub fn parse(object: std.json.ObjectMap) !Spec {
        const url = nonEmptyString(object, "url") orelse return error.InvalidEffect;
        if ((!std.mem.startsWith(u8, url, "https://") and !std.mem.startsWith(u8, url, "http://")) or std.mem.indexOfScalar(u8, url, 0) != null)
            return error.InvalidEffect;
        const method_name = if (object.get("method")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else "POST";
        const method: std.http.Method = if (std.mem.eql(u8, method_name, "POST")) .POST else if (std.mem.eql(u8, method_name, "GET")) .GET else return error.InvalidEffect;
        const body = if (object.get("body")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else null;
        const json = object.get("json");
        if (body != null and json != null) return error.InvalidEffect;
        const headers = if (object.get("headers")) |value| switch (value) {
            .array => |array| array.items,
            else => return error.InvalidEffect,
        } else &.{};
        for (headers) |value| {
            const header = switch (value) {
                .object => |item| item,
                else => return error.InvalidEffect,
            };
            try validateHeader(nonEmptyString(header, "name") orelse return error.InvalidEffect);
            try validateHeader(stringField(header, "value") orelse return error.InvalidEffect);
        }
        const credential: ?Credential = if (object.get("credential")) |value| blk: {
            const item = switch (value) {
                .object => |entry| entry,
                else => return error.InvalidEffect,
            };
            const parsed: Credential = .{
                .id = nonEmptyString(item, "id") orelse return error.InvalidEffect,
                .header = nonEmptyString(item, "header") orelse return error.InvalidEffect,
                .prefix = stringField(item, "prefix") orelse "",
                .metadata_field = stringField(item, "metadata_field"),
                .metadata_header = stringField(item, "metadata_header"),
            };
            if ((parsed.metadata_field == null) != (parsed.metadata_header == null)) return error.InvalidEffect;
            try validateHeader(parsed.header);
            try validateHeader(parsed.prefix);
            if (parsed.metadata_header) |header| try validateHeader(header);
            break :blk parsed;
        } else null;
        const response_format = if (object.get("response_format")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else "text";
        const timeouts: Timeouts = if (object.get("timeouts")) |value| blk: {
            const values = switch (value) {
                .object => |item| item,
                else => return error.InvalidEffect,
            };
            if (values.get("connect_ms") != null) return error.InvalidEffect;
            break :blk .{
                .first_byte_ms = try timeoutField(values, "first_byte_ms", 30_000),
                .idle_ms = try timeoutField(values, "idle_ms", 30_000),
                .overall_ms = try timeoutField(values, "overall_ms", 600_000),
            };
        } else .{};
        const retries: Retries = if (object.get("retries")) |value| blk: {
            const values = switch (value) {
                .object => |item| item,
                else => return error.InvalidEffect,
            };
            break :blk .{
                .attempts = try retryField(values, "attempts", 0, 5, 2),
                .backoff_ms = try retryField(values, "backoff_ms", 0, 60_000, 500),
                .max_ms = try retryField(values, "max_ms", 0, 600_000, 8_000),
            };
        } else .{};
        return .{
            .url = url,
            .method = method,
            .body = body,
            .json = json,
            .headers = headers,
            .credential = credential,
            .completion = nonEmptyString(object, "completion") orelse return error.InvalidEffect,
            .id = nonEmptyString(object, "id") orelse return error.InvalidEffect,
            .response_format = if (std.mem.eql(u8, response_format, "text")) .text else if (std.mem.eql(u8, response_format, "json")) .json else if (std.mem.eql(u8, response_format, "sse_json")) .sse_json else if (std.mem.eql(u8, response_format, "sse_json_stream")) .sse_json_stream else return error.InvalidEffect,
            .timeouts = timeouts,
            .retries = retries,
        };
    }
};

/// Receives one complete SSE data field while the HTTP socket is still open.
/// The callback must copy bytes it retains and may block to apply backpressure.
pub const StreamSink = struct {
    context: *anyopaque,
    emit: *const fn (context: *anyopaque, data: ?[]const u8) anyerror!void,
    activity: ?*const fn (context: *anyopaque) void = null,
};

pub const Activity = struct {
    context: *anyopaque,
    note: *const fn (context: *anyopaque) void,
};

pub const Result = struct {
    status: u16,
    body: []u8,

    pub fn deinit(self: Result, allocator: std.mem.Allocator) void {
        allocator.free(self.body);
    }
};

pub fn run(allocator: std.mem.Allocator, io: std.Io, store: ?*auth.Store, spec: Spec, activity: ?Activity) !Result {
    var attempt: u64 = 0;
    while (true) : (attempt += 1) {
        var response: BoundedWriter = .{ .allocator = allocator, .limit = 8 * 1024 * 1024, .activity = activity };
        var retry_after: ?u64 = null;
        const status = fetch(allocator, io, store, spec, &response.writer, &retry_after) catch |err| {
            const exceeded = response.exceeded;
            response.deinit();
            if (exceeded) return error.HttpResponseTooLarge;
            if (!canRetry(spec, err, attempt)) return err;
            try pause(io, spec, attempt, retry_after);
            continue;
        };
        if (transientStatus(status) and attempt < spec.retries.attempts) {
            const delay = retry_after;
            response.deinit();
            try pause(io, spec, attempt, delay);
            continue;
        }
        return .{ .status = status, .body = try response.toOwnedSlice() };
    }
}

const BoundedWriter = struct {
    writer: std.Io.Writer = .{ .vtable = &.{ .drain = drain }, .buffer = &.{} },
    allocator: std.mem.Allocator,
    limit: usize,
    bytes: std.ArrayList(u8) = .empty,
    exceeded: bool = false,
    activity: ?Activity = null,

    fn deinit(self: *BoundedWriter) void {
        self.bytes.deinit(self.allocator);
    }
    fn toOwnedSlice(self: *BoundedWriter) ![]u8 {
        return self.bytes.toOwnedSlice(self.allocator);
    }
    fn drain(writer: *std.Io.Writer, data: []const []const u8, splat: usize) std.Io.Writer.Error!usize {
        const self: *BoundedWriter = @alignCast(@fieldParentPtr("writer", writer));
        var consumed: usize = 0;
        if (data.len == 0) return 0;
        for (data[0 .. data.len - 1]) |part| {
            self.append(part) catch return error.WriteFailed;
            consumed += part.len;
        }
        for (0..splat) |_| {
            const part = data[data.len - 1];
            self.append(part) catch return error.WriteFailed;
            consumed += part.len;
        }
        return consumed;
    }
    fn append(self: *BoundedWriter, value: []const u8) !void {
        if (value.len != 0) if (self.activity) |activity| activity.note(activity.context);
        if (value.len > self.limit -| self.bytes.items.len) {
            self.exceeded = true;
            return error.ResponseTooLarge;
        }
        try self.bytes.appendSlice(self.allocator, value);
    }
};

/// Parse SSE framing incrementally from the response writer. `null` denotes
/// the provider's literal `[DONE]` marker; EOF is deliberately not equivalent.
pub const SseResult = struct { status: u16, error_body: []const u8, failure: ?anyerror };

pub fn runSse(allocator: std.mem.Allocator, io: std.Io, store: ?*auth.Store, spec: Spec, sink: StreamSink) !SseResult {
    var attempt: u64 = 0;
    while (true) : (attempt += 1) {
        var parser: SseWriter = .{ .allocator = allocator, .sink = sink };
        defer parser.deinit();
        var retry_after: ?u64 = null;
        const status = fetch(allocator, io, store, spec, &parser.writer, &retry_after) catch |err| {
            // Writer callbacks use WriteFailed as their transport error. Preserve
            // cancellation from a backpressured sink as cancellation of the HTTP
            // operation rather than misreporting it as an SSE parse failure.
            if (parser.canceled) return error.Canceled;
            if (parser.finished) return .{ .status = 200, .error_body = "", .failure = null };
            if (parser.failure) |failure| return failure;
            // A stream that already reached policy cannot be replayed without
            // duplicating records; that request is over.
            if (parser.delivered or !canRetry(spec, err, attempt)) return err;
            try pause(io, spec, attempt, retry_after);
            continue;
        };
        if (transientStatus(status) and !parser.delivered and attempt < spec.retries.attempts) {
            const delay = retry_after;
            try pause(io, spec, attempt, delay);
            continue;
        }
        // EOF never completes an SSE event. A server must terminate an event with
        // a blank line; partial line/record state is deliberately discarded.
        const error_body = if (status >= 200 and status < 300 and parser.failure == null)
            ""
        else
            try allocator.dupe(u8, parser.bounded_body.items);
        return .{ .status = status, .error_body = error_body, .failure = parser.failure };
    }
}

fn fetch(allocator: std.mem.Allocator, io: std.Io, store: ?*auth.Store, spec: Spec, writer: *std.Io.Writer, retry_after: ?*?u64) !u16 {
    var headers: std.ArrayList(std.http.Header) = .empty;
    defer headers.deinit(allocator);
    for (spec.headers) |value| {
        const object = value.object;
        try headers.append(allocator, .{ .name = object.get("name").?.string, .value = object.get("value").?.string });
    }
    var injected: ?[]u8 = null;
    defer if (injected) |value| allocator.free(value);
    if (spec.credential) |credential| {
        const secret = try (store orelse return error.CredentialStoreUnavailable).access(credential.id);
        injected = try std.mem.concat(allocator, u8, &.{ credential.prefix, secret });
        try headers.append(allocator, .{ .name = credential.header, .value = injected.? });
        if (credential.metadata_field) |field| {
            const value = store.?.getField(credential.id, field) orelse return error.CredentialMetadataMissing;
            try headers.append(allocator, .{ .name = credential.metadata_header.?, .value = value });
        }
    }
    const encoded = if (spec.json) |json| try std.json.Stringify.valueAlloc(allocator, json, .{}) else null;
    defer if (encoded) |value| allocator.free(value);
    var client: std.http.Client = .{ .allocator = allocator, .io = io };
    defer client.deinit();
    const payload = encoded orelse spec.body;
    const redirects: std.http.Client.Request.RedirectBehavior = if (payload == null) @enumFromInt(3) else .unhandled;
    var req = try client.request(spec.method, try std.Uri.parse(spec.url), .{
        .redirect_behavior = redirects,
        .extra_headers = headers.items,
    });
    defer req.deinit();
    if (payload) |bytes| {
        req.transfer_encoding = .{ .content_length = bytes.len };
        var body = try req.sendBodyUnflushed(&.{});
        try body.writer.writeAll(bytes);
        try body.end();
        try req.connection.?.flush();
    } else try req.sendBodiless();
    var redirect_buffer: [8 * 1024]u8 = undefined;
    var response = try req.receiveHead(&redirect_buffer);
    if (retry_after) |slot| slot.* = headerDelaySeconds(response.head);
    const decompress_buffer = try allocator.alloc(u8, switch (response.head.content_encoding) {
        .identity => 0,
        .zstd => std.compress.zstd.default_window_len,
        .deflate, .gzip => std.compress.flate.max_window_len,
        .compress => return error.UnsupportedCompressionMethod,
    });
    defer allocator.free(decompress_buffer);
    var transfer_buffer: [64]u8 = undefined;
    var decompress: std.http.Decompress = undefined;
    const reader = response.readerDecompressing(&transfer_buffer, &decompress, decompress_buffer);
    _ = reader.streamRemaining(writer) catch |err| switch (err) {
        // Zig 0.16's Client.fetch unwraps bodyErr here, but a canceled
        // socket read can leave that HTTP framing error unset (also via TLS).
        // The socket reader retains the original error, including Canceled.
        error.ReadFailed => {
            if (req.connection) |connection| {
                if (connection.stream_reader.err) |failure| return failure;
            }
            if (response.bodyErr()) |failure| return failure;
            return error.ReadFailed;
        },
        else => return err,
    };
    return @intFromEnum(response.head.status);
}

const SseWriter = struct {
    writer: std.Io.Writer = .{ .vtable = &.{ .drain = drain }, .buffer = &.{} },
    allocator: std.mem.Allocator,
    sink: StreamSink,
    line: std.ArrayList(u8) = .empty,
    record: std.ArrayList(u8) = .empty,
    bounded_body: std.ArrayList(u8) = .empty,
    total: usize = 0,
    failure: ?anyerror = null,
    canceled: bool = false,
    finished: bool = false,
    /// True once a record reached policy; such a stream cannot be retried.
    delivered: bool = false,

    fn deinit(self: *SseWriter) void {
        self.line.deinit(self.allocator);
        self.record.deinit(self.allocator);
        self.bounded_body.deinit(self.allocator);
    }

    fn drain(writer: *std.Io.Writer, data: []const []const u8, splat: usize) std.Io.Writer.Error!usize {
        const self: *SseWriter = @alignCast(@fieldParentPtr("writer", writer));
        var consumed: usize = 0;
        if (data.len == 0) return 0;
        for (data[0 .. data.len - 1]) |part| {
            self.accept(part) catch return error.WriteFailed;
            consumed += part.len;
        }
        for (0..splat) |_| {
            const part = data[data.len - 1];
            self.accept(part) catch return error.WriteFailed;
            consumed += part.len;
        }
        return consumed;
    }

    fn accept(self: *SseWriter, bytes: []const u8) !void {
        if (bytes.len != 0) if (self.sink.activity) |note| note(self.sink.context);
        self.total += bytes.len;
        if (self.total > 8 * 1024 * 1024) return error.HttpResponseTooLarge;
        const remaining = 64 * 1024 - self.bounded_body.items.len;
        try self.bounded_body.appendSlice(self.allocator, bytes[0..@min(remaining, bytes.len)]);
        self.feed(bytes) catch |err| {
            if (err == error.Canceled) self.canceled = true else if (err == error.StreamFinished) self.finished = true else self.failure = err;
            return err;
        };
    }

    fn feed(self: *SseWriter, bytes: []const u8) !void {
        for (bytes) |byte| {
            if (byte == '\n') try self.finishLine() else {
                try self.line.append(self.allocator, byte);
                if (self.line.items.len > 256 * 1024) return error.StreamRecordTooLarge;
            }
        }
    }

    fn finishLine(self: *SseWriter) !void {
        const line = std.mem.trimEnd(u8, self.line.items, "\r");
        if (line.len == 0) try self.emitRecord() else if (std.mem.startsWith(u8, line, "data:")) {
            if (self.record.items.len != 0) try self.record.append(self.allocator, '\n');
            try self.record.appendSlice(self.allocator, std.mem.trimStart(u8, line[5..], " \t"));
            if (self.record.items.len > 256 * 1024) return error.StreamRecordTooLarge;
        }
        self.line.clearRetainingCapacity();
    }

    fn emitRecord(self: *SseWriter) !void {
        const value = std.mem.trim(u8, self.record.items, " \t\r\n");
        if (value.len != 0) {
            try self.sink.emit(self.sink.context, if (std.mem.eql(u8, value, "[DONE]")) null else value);
            self.delivered = true;
        }
        self.record.clearRetainingCapacity();
    }
};

/// Seconds from a Retry-After header, when the server used the delay form. An
/// HTTP-date is ignored; the retry then uses the configured backoff.
fn headerDelaySeconds(head: std.http.Client.Response.Head) ?u64 {
    var iterator = head.iterateHeaders();
    while (iterator.next()) |header| {
        if (!std.ascii.eqlIgnoreCase(header.name, "retry-after")) continue;
        const trimmed = std.mem.trim(u8, header.value, " \t");
        const seconds = std.fmt.parseInt(u64, trimmed, 10) catch return null;
        // A day is already far past any useful wait; clamp rather than trust.
        return @min(seconds, 86_400);
    }
    return null;
}

fn retryField(object: std.json.ObjectMap, name: []const u8, minimum: i64, maximum: i64, default: i64) !u64 {
    const value = object.get(name) orelse return @intCast(default);
    if (value != .integer or value.integer < minimum or value.integer > maximum) return error.InvalidEffect;
    return @intCast(value.integer);
}

/// Statuses that say "ask again", not "this request is wrong".
fn transientStatus(status: u16) bool {
    return switch (status) {
        408, 425, 429, 500, 502, 503, 504 => true,
        else => false,
    };
}

/// Transport failures that leave the request itself unjudged. Cancellation and
/// timeouts are deliberately excluded: they are the operator's decision.
fn retryableError(failure: anyerror) bool {
    return switch (failure) {
        error.ConnectionRefused,
        error.ConnectionResetByPeer,
        error.NetworkUnreachable,
        error.NetworkSubsystemFailed,
        error.TemporaryNameServerFailure,
        error.ReadFailed,
        => true,
        else => false,
    };
}

fn canRetry(spec: Spec, failure: anyerror, attempt: u64) bool {
    return attempt < spec.retries.attempts and retryableError(failure);
}

/// Milliseconds to wait before another attempt: exponential backoff, a
/// Retry-After delay when the server sent one, and the configured ceiling.
fn pauseDelay(spec: Spec, attempt: u64, retry_after_s: ?u64) u64 {
    const shifted = spec.retries.backoff_ms * (@as(u64, 1) << @intCast(@min(attempt, 16)));
    return @min(if (retry_after_s) |seconds| seconds * 1000 else shifted, spec.retries.max_ms);
}

/// Wait before the next attempt. The wait is cancellable.
fn pause(io: std.Io, spec: Spec, attempt: u64, retry_after_s: ?u64) !void {
    const delay = pauseDelay(spec, attempt, retry_after_s);
    if (delay == 0) return;
    std.Io.sleep(io, .fromMilliseconds(@intCast(delay)), .awake) catch return error.Canceled;
}

fn timeoutField(object: std.json.ObjectMap, name: []const u8, default: u64) !u64 {
    const value = object.get(name) orelse return default;
    const integer = switch (value) {
        .integer => |item| item,
        else => return error.InvalidEffect,
    };
    if (integer < 1 or integer > 3_600_000) return error.InvalidEffect;
    return @intCast(integer);
}

fn validateHeader(value: []const u8) !void {
    if (std.mem.indexOfAny(u8, value, "\r\n\x00") != null) return error.InvalidEffect;
}

fn stringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |string| string,
        else => null,
    };
}

fn nonEmptyString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = stringField(object, name) orelse return null;
    return if (value.len == 0 or std.mem.indexOfScalar(u8, value, 0) != null) null else value;
}

test "retry policy bounds attempts and honors Retry-After" {
    const base: Spec = .{ .url = "https://example.test", .method = .POST, .body = null, .json = null, .headers = &.{}, .credential = null, .completion = "done", .id = "1", .response_format = .text };
    try std.testing.expect(transientStatus(429) and transientStatus(503) and transientStatus(408));
    try std.testing.expect(!transientStatus(200) and !transientStatus(401) and !transientStatus(404));
    try std.testing.expect(retryableError(error.ConnectionRefused));
    try std.testing.expect(retryableError(error.ReadFailed));
    try std.testing.expect(!retryableError(error.Canceled));
    try std.testing.expect(!retryableError(error.CredentialStoreUnavailable));
    try std.testing.expect(canRetry(base, error.ConnectionRefused, 0));
    try std.testing.expect(canRetry(base, error.ConnectionRefused, base.retries.attempts - 1));
    try std.testing.expect(!canRetry(base, error.ConnectionRefused, base.retries.attempts));
    const none: Spec = .{ .url = base.url, .method = base.method, .body = null, .json = null, .headers = &.{}, .credential = null, .completion = "done", .id = "1", .response_format = .text, .retries = .{ .attempts = 0 } };
    try std.testing.expect(!canRetry(none, error.ConnectionRefused, 0));

    // A delay never exceeds the ceiling, whatever the server asks for.
    try std.testing.expectEqual(@as(u64, 500), pauseDelay(base, 0, null));
    try std.testing.expectEqual(@as(u64, 1000), pauseDelay(base, 1, null));
    try std.testing.expectEqual(@as(u64, 8000), pauseDelay(base, 9, null));
    try std.testing.expectEqual(@as(u64, 3000), pauseDelay(base, 0, 3));
    try std.testing.expectEqual(@as(u64, 8000), pauseDelay(base, 0, 60));
}

test "retry declarations are validated" {
    const parse = struct {
        fn spec(json: []const u8) !Spec {
            var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, json, .{});
            defer parsed.deinit();
            return Spec.parse(parsed.value.object);
        }
    }.spec;
    const accepted = try parse("{\"url\":\"https://example.test\",\"completion\":\"c\",\"id\":\"1\",\"retries\":{\"attempts\":4,\"backoff_ms\":10,\"max_ms\":20}}");
    try std.testing.expectEqual(@as(u64, 4), accepted.retries.attempts);
    try std.testing.expectEqual(@as(u64, 10), accepted.retries.backoff_ms);
    try std.testing.expectEqual(@as(u64, 20), accepted.retries.max_ms);
    const default_retries = try parse("{\"url\":\"https://example.test\",\"completion\":\"c\",\"id\":\"1\"}");
    try std.testing.expectEqual(@as(u64, 2), default_retries.retries.attempts);
    try std.testing.expectError(error.InvalidEffect, parse("{\"url\":\"https://example.test\",\"completion\":\"c\",\"id\":\"1\",\"retries\":{\"attempts\":9}}"));
    try std.testing.expectError(error.InvalidEffect, parse("{\"url\":\"https://example.test\",\"completion\":\"c\",\"id\":\"1\",\"retries\":{\"attempts\":-1}}"));
    try std.testing.expectError(error.InvalidEffect, parse("{\"url\":\"https://example.test\",\"completion\":\"c\",\"id\":\"1\",\"retries\":5}"));
}

test "HTTP timeout contract uses observable first-byte idle and overall phases" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator,
        \\{"url":"https://example.test","completion":"done","id":"1","timeouts":{"first_byte_ms":12,"idle_ms":34,"overall_ms":56}}
    , .{});
    defer parsed.deinit();
    const spec = try Spec.parse(parsed.value.object);
    try std.testing.expectEqual(@as(u64, 12), spec.timeouts.first_byte_ms);
    try std.testing.expectEqual(@as(u64, 34), spec.timeouts.idle_ms);
    try std.testing.expectEqual(@as(u64, 56), spec.timeouts.overall_ms);

    var obsolete = try std.json.parseFromSlice(std.json.Value, std.testing.allocator,
        \\{"url":"https://example.test","completion":"done","id":"1","timeouts":{"connect_ms":10}}
    , .{});
    defer obsolete.deinit();
    try std.testing.expectError(error.InvalidEffect, Spec.parse(obsolete.value.object));
}

test "buffered HTTP writer reports body activity before completion" {
    const Probe = struct {
        count: usize = 0,
        fn note(context: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(context));
            self.count += 1;
        }
    };
    var probe: Probe = .{};
    var writer: BoundedWriter = .{ .allocator = std.testing.allocator, .limit = 16, .activity = .{ .context = &probe, .note = Probe.note } };
    defer writer.deinit();
    try writer.append("body");
    try std.testing.expectEqual(@as(usize, 1), probe.count);
}

test "regular HTTP response writer rejects before crossing its allocation bound" {
    var writer: BoundedWriter = .{ .allocator = std.testing.allocator, .limit = 4 };
    defer writer.deinit();
    try writer.append("1234");
    try std.testing.expectError(error.ResponseTooLarge, writer.append("5"));
    try std.testing.expectEqual(@as(usize, 4), writer.bytes.items.len);
    try std.testing.expect(writer.exceeded);
}

test "SSE EOF drops an unterminated event" {
    const Probe = struct {
        count: usize = 0,
        fn emit(context: *anyopaque, _: ?[]const u8) anyerror!void {
            const self: *@This() = @ptrCast(@alignCast(context));
            self.count += 1;
        }
    };
    var probe: Probe = .{};
    var parser: SseWriter = .{ .allocator = std.testing.allocator, .sink = .{ .context = &probe, .emit = Probe.emit } };
    defer parser.deinit();
    try parser.accept("data: {\"partial\":true}\n");
    try std.testing.expectEqual(@as(usize, 0), probe.count);
    try parser.accept("\n");
    try std.testing.expectEqual(@as(usize, 1), probe.count);
    try parser.accept("data: {\"unterminated\":true}");
    try std.testing.expectEqual(@as(usize, 1), probe.count);
}

test "SSE sink cancellation aborts the writer without becoming parse failure" {
    const Probe = struct {
        fn emit(_: *anyopaque, _: ?[]const u8) anyerror!void {
            return error.Canceled;
        }
    };
    var ignored: u8 = 0;
    var parser: SseWriter = .{ .allocator = std.testing.allocator, .sink = .{ .context = &ignored, .emit = Probe.emit } };
    defer parser.deinit();
    try std.testing.expectError(error.Canceled, parser.accept("data: {}\n\n"));
    try std.testing.expect(parser.canceled);
    try std.testing.expect(parser.failure == null);
}

test "SSE terminal marker aborts parsing immediately" {
    const Probe = struct {
        fn emit(_: *anyopaque, data: ?[]const u8) anyerror!void {
            if (data == null) return error.StreamFinished;
        }
    };
    var ignored: u8 = 0;
    var parser: SseWriter = .{ .allocator = std.testing.allocator, .sink = .{ .context = &ignored, .emit = Probe.emit } };
    defer parser.deinit();
    try std.testing.expectError(error.StreamFinished, parser.accept("data: [DONE]\n\n"));
    try std.testing.expect(parser.finished);
}

test "HTTP effect validation keeps credentials referential" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator,
        \\{"type":"http/request","url":"https://example.test/v1","headers":[{"name":"content-type","value":"application/json"}],"credential":{"id":"openai","header":"authorization","prefix":"Bearer "},"completion":"done","id":"1","body":"{}"}
    , .{});
    defer parsed.deinit();
    const spec = try Spec.parse(parsed.value.object);
    try std.testing.expectEqualStrings("openai", spec.credential.?.id);
    try std.testing.expectEqual(std.http.Method.POST, spec.method);
}

/// Live composition owns credential resolution alongside the transport. Fixture
/// applications replace this dependency before either capability is acquired.
pub fn request(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, spec: Spec, activity: ?Activity) !Result {
    var store = if (spec.credential != null) try auth.Store.init(allocator, io, environ) else null;
    defer if (store) |*value| value.deinit();
    if (spec.credential) |credential| try auth.validateCredentialOrigin(credential.id, spec.url, if (store) |*value| value else null, environ);
    return run(allocator, io, if (store) |*value| value else null, spec, activity);
}

pub fn requestSse(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, spec: Spec, sink: StreamSink) !SseResult {
    var store = if (spec.credential != null) try auth.Store.init(allocator, io, environ) else null;
    defer if (store) |*value| value.deinit();
    if (spec.credential) |credential| try auth.validateCredentialOrigin(credential.id, spec.url, if (store) |*value| value else null, environ);
    return runSse(allocator, io, if (store) |*value| value else null, spec, sink);
}
