//! Concurrent native operation collection, fair scheduling, and cancellation.
const std = @import("std");
const posix = std.posix;
const auth = @import("misa_auth");
const file = @import("misa_file");
const image = @import("misa_image");
const process = @import("misa_process");
const http = @import("http.zig");
const channel = @import("operation/channel.zig");
const result_json = @import("operation/result_json.zig");
const operation_task = @import("operation/task.zig");

pub const Item = result_json.Item;
const Task = operation_task.Task;

pub const Owner = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    active: std.ArrayList(*Task) = .empty,
    cursor: usize = 0,
    serial: u64 = 0,
    wakeup: channel.Wakeup,

    pub fn init(allocator: std.mem.Allocator, io: std.Io) !Owner {
        return .{ .allocator = allocator, .io = io, .wakeup = try .init() };
    }

    pub fn deinit(self: *Owner) void {
        for (self.active.items) |task| {
            task.cancel();
            task.destroy();
        }
        self.active.deinit(self.allocator);
        self.wakeup.deinit();
    }

    pub fn wakeupFd(self: *const Owner) posix.fd_t {
        return self.wakeup.read_fd;
    }

    pub fn consumeWakeup(self: *Owner) void {
        self.wakeup.consume();
    }

    pub fn isActive(self: *const Owner) bool {
        return self.active.items.len != 0;
    }

    pub fn startProcess(self: *Owner, source: process.Spec) !void {
        try self.ensureUnique(source.id);
        try self.active.ensureUnusedCapacity(self.allocator, 1);
        const task = try Task.createProcess(self.allocator, self.io, self.wakeup, source);
        self.startPrepared(task);
    }

    pub fn startHttp(self: *Owner, source: http.Spec, environ: *const std.process.Environ.Map) !void {
        try self.ensureUnique(source.id);
        try self.active.ensureUnusedCapacity(self.allocator, 1);
        const task = try Task.createHttp(self.allocator, self.io, self.wakeup, source, environ);
        self.startPrepared(task);
    }

    pub fn startFile(self: *Owner, source: file.Spec) !void {
        try self.ensureUnique(source.requestId());
        try self.active.ensureUnusedCapacity(self.allocator, 1);
        const task = try Task.createFile(self.allocator, self.io, self.wakeup, source);
        self.startPrepared(task);
    }

    pub fn startImage(self: *Owner, source: image.Spec, environ: *const std.process.Environ.Map) !void {
        try self.ensureUnique(source.id);
        try self.active.ensureUnusedCapacity(self.allocator, 1);
        const task = try Task.createImage(self.allocator, self.io, self.wakeup, source, environ);
        self.startPrepared(task);
    }

    pub fn startStateLoad(self: *Owner, namespace: []const u8, completion: []const u8, environ: *const std.process.Environ.Map) !void {
        try self.startState(namespace, completion, null, environ);
    }

    pub fn startStateSave(self: *Owner, namespace: []const u8, data: std.json.Value, environ: *const std.process.Environ.Map) !void {
        try self.startState(namespace, "state/saved", data, environ);
    }

    fn startState(self: *Owner, namespace: []const u8, completion: []const u8, data: ?std.json.Value, environ: *const std.process.Environ.Map) !void {
        try self.active.ensureUnusedCapacity(self.allocator, 1);
        const serial = self.serial +% 1;
        const task = try Task.createState(self.allocator, self.io, self.wakeup, namespace, completion, serial, data, environ);
        self.serial = serial;
        self.startPrepared(task);
    }

    pub fn startAuth(self: *Owner, action: auth.Action, declaration: auth.Declaration, completion: []const u8, interaction: []const u8, id: []const u8, environ: *const std.process.Environ.Map, terminal_lease: ?u64, managed_input: bool) !void {
        try self.ensureUnique(id);
        try self.active.ensureUnusedCapacity(self.allocator, 1);
        const task = try Task.createAuth(self.allocator, self.io, self.wakeup, action, declaration, completion, interaction, id, environ, terminal_lease, managed_input);
        self.startPrepared(task);
    }

    /// Return at most one item, checking operations in round-robin order.
    pub fn pop(self: *Owner, now_ns: i96) !?Item {
        if (self.active.items.len == 0) return null;
        var checked: usize = 0;
        while (checked < self.active.items.len) : (checked += 1) {
            if (self.cursor >= self.active.items.len) self.cursor = 0;
            const index = self.cursor;
            self.cursor = (self.cursor + 1) % self.active.items.len;
            const task = self.active.items[index];
            if (task.timeoutName(now_ns)) |name| return self.removeCanceled(index, name, false);
            if (try task.popRecord()) |item| return item;
            if (task.isDone()) return try self.removeCompleted(index);
        }
        return null;
    }

    pub fn hasReady(self: *Owner, now_ns: i96) bool {
        for (self.active.items) |task| if (task.isDone() or task.recordCount() != 0 or task.timeoutName(now_ns) != null) return true;
        return false;
    }

    pub fn nextDeadline(self: *const Owner) ?i96 {
        var result: ?i96 = null;
        for (self.active.items) |task| {
            const deadline = task.nextDeadline();
            if (result == null or deadline < result.?) result = deadline;
        }
        return result;
    }

    pub fn respond(self: *Owner, id: []const u8, correlation: []const u8, action: []const u8, value: []const u8) !void {
        const index = self.find(id) orelse return error.OperationNotFound;
        try self.active.items[index].respond(correlation, action, value);
    }

    pub fn expectsInput(self: *Owner, id: []const u8, correlation: []const u8) bool {
        const index = self.find(id) orelse return false;
        return self.active.items[index].expectsInput(correlation);
    }

    pub fn cancel(self: *Owner, id: []const u8) !?Item {
        const index = self.find(id) orelse return null;
        return self.removeCanceled(index, "Canceled", false);
    }

    /// Stop a peer after policy observes its semantic terminal record.
    pub fn finish(self: *Owner, id: []const u8) !?Item {
        const index = self.find(id) orelse return null;
        return self.removeCanceled(index, null, true);
    }

    fn startPrepared(self: *Owner, task: *Task) void {
        task.start();
        self.active.appendAssumeCapacity(task);
    }

    fn ensureUnique(self: *Owner, id: []const u8) !void {
        if (self.find(id) != null) return error.DuplicateOperationId;
    }

    pub fn find(self: *const Owner, id: []const u8) ?usize {
        for (self.active.items, 0..) |task, index| if (std.mem.eql(u8, task.id, id)) return index;
        return null;
    }

    fn removeCompleted(self: *Owner, index: usize) !Item {
        const task = self.active.items[index];
        try task.await();
        const item = task.completionItem() catch task.takeFallback();
        self.remove(index);
        task.destroy();
        return item;
    }

    fn removeCanceled(self: *Owner, index: usize, message: ?[]const u8, success: bool) Item {
        const task = self.active.items[index];
        task.cancel();
        task.discardRecords();
        const item = task.forcedItem(success, message) catch task.takeFallback();
        self.remove(index);
        task.destroy();
        return item;
    }

    fn remove(self: *Owner, index: usize) void {
        _ = self.active.orderedRemove(index);
        if (self.cursor > index) self.cursor -= 1;
    }
};

test "operations run concurrently and semantic finish kills a sleeping process" {
    var owner: Owner = try .init(std.testing.allocator, std.testing.io);
    defer owner.deinit();
    const argv_a = [_]std.json.Value{ .{ .string = "/bin/sh" }, .{ .string = "-c" }, .{ .string = "sleep 0.15; printf done" } };
    const argv_b = [_]std.json.Value{ .{ .string = "/bin/sh" }, .{ .string = "-c" }, .{ .string = "sleep 0.15; printf done" } };
    const started = std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds;
    try owner.startProcess(.{ .argv = &argv_a, .completion = "done", .id = "a", .stdout_format = .text, .stdin = null, .stdin_json = null });
    try owner.startProcess(.{ .argv = &argv_b, .completion = "done", .id = "b", .stdout_format = .text, .stdin = null, .stdin_json = null });
    var completed: usize = 0;
    while (completed < 2) {
        if (try owner.pop(std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds)) |item| {
            defer std.testing.allocator.free(item.json);
            completed += 1;
        } else std.Io.sleep(std.testing.io, .fromMilliseconds(5), .awake) catch {};
    }
    const elapsed = std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds - started;
    try std.testing.expect(elapsed < 280 * std.time.ns_per_ms);

    const stream_argv = [_]std.json.Value{ .{ .string = "/bin/sh" }, .{ .string = "-c" }, .{ .string = "printf '{\"type\":\"result\"}\\n'; sleep 2" } };
    try owner.startProcess(.{ .argv = &stream_argv, .completion = "stream", .id = "sleeping", .stdout_format = .json_lines_stream, .stdin = null, .stdin_json = null });
    while (true) {
        if (try owner.pop(std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds)) |item| {
            defer std.testing.allocator.free(item.json);
            if (!item.terminal) break;
        } else std.Io.sleep(std.testing.io, .fromMilliseconds(5), .awake) catch {};
    }
    const finish_started = std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds;
    const finished = (try owner.finish("sleeping")).?;
    defer std.testing.allocator.free(finished.json);
    try std.testing.expect(std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds - finish_started < 500 * std.time.ns_per_ms);
    try std.testing.expect(!owner.isActive());
}

test "image operations load files and configured clipboard commands asynchronously" {
    const a = std.testing.allocator;
    const encoded = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAADklEQVR4nGM4kSLyH4QBE+sEf0CWJyAAAAAASUVORK5CYII=";
    const bytes = try a.alloc(u8, try std.base64.standard.Decoder.calcSizeForSlice(encoded));
    defer a.free(bytes);
    try std.base64.standard.Decoder.decode(bytes, encoded);
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    try tmp.dir.writeFile(std.testing.io, .{ .sub_path = "sample.png", .data = bytes });
    const path = try std.fmt.allocPrint(a, ".zig-cache/tmp/{s}/sample.png", .{tmp.sub_path});
    defer a.free(path);
    var environ = std.process.Environ.Map.init(a);
    defer environ.deinit();
    var owner = try Owner.init(a, std.testing.io);
    defer owner.deinit();
    try owner.startImage(.{ .path = path, .completion = "image/loaded", .id = "file" }, &environ);
    const argv = [_]std.json.Value{ .{ .string = "cat" }, .{ .string = path } };
    try owner.startImage(.{ .argv = &argv, .completion = "image/loaded", .id = "clipboard" }, &environ);
    var completed: usize = 0;
    while (completed < 2) {
        if (try owner.pop(std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds)) |item| {
            defer a.free(item.json);
            var parsed = try std.json.parseFromSlice(std.json.Value, a, item.json, .{});
            defer parsed.deinit();
            const event = parsed.value.object;
            if (!event.get("ok").?.bool) std.debug.print("image outcome: {s}\n", .{item.json});
            try std.testing.expect(event.get("ok").?.bool);
            const data = event.get("data").?.object;
            try std.testing.expectEqualStrings(encoded, data.get("data").?.string);
            try std.testing.expectEqual(@as(i64, 2), data.get("width").?.integer);
            try std.testing.expectEqualStrings("rgba", data.get("preview").?.object.get("format").?.string);
            try std.testing.expectEqualStrings("yGQU/8hkFP8=", data.get("preview").?.object.get("data").?.string);
            completed += 1;
        } else std.Io.sleep(std.testing.io, .fromMilliseconds(1), .awake) catch {};
    }
}
