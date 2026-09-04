//! Ownership, scheduling, and bounded publication for one streaming operation.
const std = @import("std");
const auth = @import("misa_auth");
const http = @import("http.zig");
const process = @import("misa_process");

pub const Item = struct {
    json: []u8,
    terminal: bool = false,
};

pub const Owner = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    active: ?*Operation = null,

    pub fn init(allocator: std.mem.Allocator, io: std.Io) Owner {
        return .{ .allocator = allocator, .io = io };
    }

    pub fn deinit(self: *Owner) void {
        if (self.active) |operation| {
            operation.group.cancel(self.io);
            self.destroy(operation);
            self.active = null;
        }
    }

    pub fn isActive(self: *const Owner) bool {
        return self.active != null;
    }

    /// Construction is private and publication is the final fallible-free step,
    /// so callers can never observe an operation destroyed by an errdefer.
    pub fn startProcess(self: *Owner, source: process.Spec) !void {
        if (self.active != null) return error.OperationAlreadyActive;
        const operation = try self.create();
        errdefer self.destroy(operation);
        const allocator = operation.arena.allocator();
        const argv = try allocator.alloc(std.json.Value, source.argv.len);
        for (source.argv, argv) |arg, *copy| copy.* = .{ .string = try allocator.dupe(u8, arg.string) };
        const spec: process.Spec = .{
            .argv = argv,
            .completion = try allocator.dupe(u8, source.completion),
            .id = try allocator.dupe(u8, source.id),
            .stdout_format = .json_lines_stream,
            .stdin = if (source.stdin) |value| try allocator.dupe(u8, value) else null,
            .stdin_json = if (source.stdin_json) |value| try cloneJson(allocator, value) else null,
        };
        operation.kind = .{ .process = spec };
        try operation.prepare(spec.completion, spec.id);
        operation.group.async(self.io, Operation.worker, .{operation});
        self.active = operation;
    }

    pub fn startHttp(self: *Owner, source: http.Spec, store: ?*auth.Store) !void {
        if (self.active != null) return error.OperationAlreadyActive;
        const operation = try self.create();
        errdefer self.destroy(operation);
        const allocator = operation.arena.allocator();
        const extra: usize = if (source.credential) |credential| if (credential.metadata_field != null) 2 else 1 else 0;
        const headers = try allocator.alloc(std.json.Value, source.headers.len + extra);
        for (source.headers, headers[0..source.headers.len]) |value, *copy| copy.* = try cloneJson(allocator, value);
        if (source.credential) |credential| {
            const credential_store = store orelse return error.CredentialStoreUnavailable;
            const secret = try credential_store.access(credential.id);
            const value = try std.mem.concat(allocator, u8, &.{ credential.prefix, secret });
            headers[source.headers.len] = try headerJson(allocator, credential.header, value);
            if (credential.metadata_field) |field| headers[source.headers.len + 1] = try headerJson(
                allocator,
                credential.metadata_header.?,
                credential_store.getField(credential.id, field) orelse return error.CredentialMetadataMissing,
            );
        }
        const spec: http.Spec = .{
            .url = try allocator.dupe(u8, source.url),
            .method = source.method,
            .body = if (source.body) |value| try allocator.dupe(u8, value) else null,
            .json = if (source.json) |value| try cloneJson(allocator, value) else null,
            .headers = headers,
            .credential = null,
            .completion = try allocator.dupe(u8, source.completion),
            .id = try allocator.dupe(u8, source.id),
            .response_format = .sse_json_stream,
        };
        operation.kind = .{ .http = spec };
        try operation.prepare(spec.completion, spec.id);
        operation.group.async(self.io, Operation.worker, .{operation});
        self.active = operation;
    }

    /// Returns queued records first. Completion is synthesized only after the
    /// worker has published its outcome and every preceding record is drained.
    pub fn pop(self: *Owner) !?Item {
        const operation = self.active orelse return null;
        if (try operation.popRecord()) |item| return item;
        if (!operation.done.load(.acquire)) return null;
        operation.group.await(self.io) catch |err| if (err != error.Canceled) return err;
        const item = operation.completionItem() catch operation.takeFallback();
        self.destroy(operation);
        self.active = null;
        return item;
    }

    pub fn cancel(self: *Owner, id: []const u8) !?Item {
        const operation = self.active orelse return null;
        if (!std.mem.eql(u8, operation.id, id)) return null;
        operation.group.cancel(self.io);
        operation.discardRecords();
        const item = operation.canceledItem() catch operation.takeFallback();
        self.destroy(operation);
        self.active = null;
        return item;
    }

    fn create(self: *Owner) !*Operation {
        const operation = try self.allocator.create(Operation);
        operation.* = .{
            .owner_allocator = self.allocator,
            .arena = .init(self.allocator),
            .io = self.io,
            .kind = undefined,
        };
        return operation;
    }

    fn destroy(self: *Owner, operation: *Operation) void {
        operation.discardRecords();
        if (operation.fallback) |json| self.allocator.free(json);
        operation.arena.deinit();
        self.allocator.destroy(operation);
    }
};

const Operation = struct {
    const capacity = max_records_per_event;
    pub const max_records_per_event = 32;
    const Kind = union(enum) { http: http.Spec, process: process.Spec };
    const QueuedRecord = union(enum) {
        data: []u8,
        terminal,
    };
    const Outcome = struct { ok: bool = false, status: i64 = 0, body: []const u8 = "", message: ?[]const u8 = "OperationFailed" };

    owner_allocator: std.mem.Allocator,
    arena: std.heap.ArenaAllocator,
    io: std.Io,
    kind: Kind,
    completion: []const u8 = "",
    id: []const u8 = "",
    fallback: ?[]u8 = null,
    outcome: Outcome = .{},
    done: std.atomic.Value(bool) = .init(false),
    group: std.Io.Group = .init,
    mutex: std.Io.Mutex = .init,
    not_full: std.Io.Condition = .init,
    items: [capacity]?QueuedRecord = .{null} ** capacity,
    head: usize = 0,
    len: usize = 0,

    fn prepare(self: *Operation, completion: []const u8, id: []const u8) !void {
        self.completion = completion;
        self.id = id;
        // This reserved event makes worker allocation and queue failures
        // terminal rather than leaving the scheduler waiting forever.
        self.fallback = try terminalJson(self.owner_allocator, completion, id, false, 0, "", "OperationFailed");
    }

    fn pushRecord(self: *Operation, record: QueuedRecord) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        while (self.len == capacity) try self.not_full.wait(self.io, &self.mutex);
        self.items[(self.head + self.len) % capacity] = record;
        self.len += 1;
    }

    /// Take one immediately available ordered batch. The consumer polling
    /// cadence supplies the latency bound; a full batch, terminal marker, or
    /// worker completion supplies the structural flush bound.
    fn popRecord(self: *Operation) !?Item {
        var records: [max_records_per_event][]u8 = undefined;
        var record_count: usize = 0;
        var terminal_marker = false;

        self.mutex.lockUncancelable(self.io);
        while (self.len != 0 and record_count < max_records_per_event) {
            const queued = self.items[self.head].?;
            switch (queued) {
                .data => |json| {
                    records[record_count] = json;
                    record_count += 1;
                },
                .terminal => terminal_marker = true,
            }
            self.items[self.head] = null;
            self.head = (self.head + 1) % capacity;
            self.len -= 1;
            if (terminal_marker) break;
        }
        if (record_count != 0 or terminal_marker) self.not_full.signal(self.io);
        self.mutex.unlock(self.io);
        if (record_count == 0 and !terminal_marker) return null;
        defer for (records[0..record_count]) |json| self.owner_allocator.free(json);
        return .{ .json = try dataJson(self.owner_allocator, self.completion, self.id, records[0..record_count], terminal_marker) };
    }

    fn discardRecords(self: *Operation) void {
        while (true) {
            self.mutex.lockUncancelable(self.io);
            if (self.len == 0) {
                self.mutex.unlock(self.io);
                return;
            }
            const queued = self.items[self.head].?;
            self.items[self.head] = null;
            self.head = (self.head + 1) % capacity;
            self.len -= 1;
            self.not_full.signal(self.io);
            self.mutex.unlock(self.io);
            switch (queued) {
                .data => |json| self.owner_allocator.free(json),
                .terminal => {},
            }
        }
    }

    fn emitHttpRecord(context: *anyopaque, raw: ?[]const u8) anyerror!void {
        const self: *Operation = @ptrCast(@alignCast(context));
        if (raw == null) return self.pushRecord(.terminal);
        var parsed = try std.json.parseFromSlice(std.json.Value, self.owner_allocator, raw.?, .{});
        defer parsed.deinit();
        const owned = try self.owner_allocator.dupe(u8, raw.?);
        errdefer self.owner_allocator.free(owned);
        try self.pushRecord(.{ .data = owned });
    }

    fn emitProcessRecord(context: *anyopaque, raw: []const u8) anyerror!void {
        return emitHttpRecord(context, raw);
    }

    fn worker(self: *Operation) std.Io.Cancelable!void {
        defer self.done.store(true, .release);
        switch (self.kind) {
            .http => |spec| {
                const result = http.runSse(self.arena.allocator(), self.io, null, spec, .{ .context = self, .emit = emitHttpRecord }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.outcome = .{ .message = @errorName(err) };
                    return;
                };
                const body = process.sanitizeOutput(self.arena.allocator(), result.error_body, 64 * 1024) catch |err| {
                    self.outcome = .{ .status = result.status, .message = @errorName(err) };
                    return;
                };
                self.outcome = .{
                    .ok = result.status >= 200 and result.status < 300 and result.failure == null,
                    .status = result.status,
                    .body = body,
                    .message = if (result.failure) |failure| @errorName(failure) else null,
                };
            },
            .process => |spec| {
                const result = process.runJsonLines(self.arena.allocator(), self.io, spec, .{ .context = self, .emit = emitProcessRecord }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.outcome = .{ .status = -1, .message = @errorName(err) };
                    return;
                };
                self.outcome = .{ .ok = result.status == 0, .status = result.status, .body = result.stderr, .message = null };
            },
        }
    }

    fn completionItem(self: *Operation) !Item {
        const json = try terminalJson(self.owner_allocator, self.completion, self.id, self.outcome.ok, self.outcome.status, self.outcome.body, self.outcome.message);
        self.owner_allocator.free(self.fallback.?);
        self.fallback = null;
        return .{ .json = json, .terminal = true };
    }

    fn canceledItem(self: *Operation) !Item {
        const json = try terminalJson(self.owner_allocator, self.completion, self.id, false, 0, "", "Canceled");
        self.owner_allocator.free(self.fallback.?);
        self.fallback = null;
        return .{ .json = json, .terminal = true };
    }

    fn takeFallback(self: *Operation) Item {
        const json = self.fallback.?;
        self.fallback = null;
        return .{ .json = json, .terminal = true };
    }
};

fn dataJson(allocator: std.mem.Allocator, completion: []const u8, id: []const u8, records: []const []u8, terminal_marker: bool) ![]u8 {
    const encoded_type = try std.json.Stringify.valueAlloc(allocator, completion, .{});
    defer allocator.free(encoded_type);
    const encoded_id = try std.json.Stringify.valueAlloc(allocator, id, .{});
    defer allocator.free(encoded_id);
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    try out.print(allocator, "{{\"type\":{s},\"id\":{s},\"phase\":\"data\",\"records\":[", .{ encoded_type, encoded_id });
    for (records, 0..) |record, index| {
        if (index != 0) try out.append(allocator, ',');
        try out.appendSlice(allocator, record);
    }
    try out.print(allocator, "],\"terminal\":{s}}}", .{if (terminal_marker) "true" else "false"});
    return out.toOwnedSlice(allocator);
}

fn terminalJson(allocator: std.mem.Allocator, completion: []const u8, id: []const u8, ok: bool, status: i64, body: []const u8, message: ?[]const u8) ![]u8 {
    return std.json.Stringify.valueAlloc(allocator, .{
        .type = completion,
        .id = id,
        .phase = "end",
        .ok = ok,
        .status = status,
        .body = body,
        .message = message,
    }, .{});
}

fn cloneJson(allocator: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    const encoded = try std.json.Stringify.valueAlloc(allocator, value, .{});
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, encoded, .{ .allocate = .alloc_always });
    return parsed.value;
}

fn headerJson(allocator: std.mem.Allocator, name: []const u8, value: []const u8) !std.json.Value {
    const encoded = try std.json.Stringify.valueAlloc(allocator, .{ .name = name, .value = value }, .{});
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, encoded, .{ .allocate = .alloc_always });
    return parsed.value;
}

test "ordered records coalesce to 32 and terminal markers flush" {
    var owner: Owner = .init(std.testing.allocator, std.testing.io);
    defer owner.deinit();
    const operation = try owner.create();
    owner.active = operation;
    operation.kind = undefined;
    try operation.prepare("stream/done", "quoted-\"id");

    for (0..Operation.max_records_per_event) |index| {
        const json = try std.fmt.allocPrint(std.testing.allocator, "{{\"index\":{d}}}", .{index});
        try operation.pushRecord(.{ .data = json });
    }

    const first = (try operation.popRecord()).?;
    defer std.testing.allocator.free(first.json);
    var first_parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, first.json, .{});
    defer first_parsed.deinit();
    try std.testing.expectEqual(Operation.max_records_per_event, first_parsed.value.object.get("records").?.array.items.len);
    try std.testing.expectEqual(@as(i64, 0), first_parsed.value.object.get("records").?.array.items[0].object.get("index").?.integer);
    try std.testing.expect(!first_parsed.value.object.get("terminal").?.bool);

    for (Operation.max_records_per_event..35) |index| {
        const json = try std.fmt.allocPrint(std.testing.allocator, "{{\"index\":{d}}}", .{index});
        try operation.pushRecord(.{ .data = json });
    }
    try operation.pushRecord(.terminal);
    const second = (try operation.popRecord()).?;
    defer std.testing.allocator.free(second.json);
    var second_parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, second.json, .{});
    defer second_parsed.deinit();
    const tail = second_parsed.value.object.get("records").?.array.items;
    try std.testing.expectEqual(@as(usize, 3), tail.len);
    try std.testing.expectEqual(@as(i64, 32), tail[0].object.get("index").?.integer);
    try std.testing.expect(second_parsed.value.object.get("terminal").?.bool);
    try std.testing.expectEqualStrings("quoted-\"id", second_parsed.value.object.get("id").?.string);
}
