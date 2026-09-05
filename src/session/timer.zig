//! Owned timer registrations and deterministic deadline scheduling.
const std = @import("std");

pub const Start = struct { interval_ms: u64, completion: []const u8, id: []const u8 };
pub const Stop = struct { id: []const u8 };

pub const Tick = struct {
    completion: []const u8,
    id: []const u8,
    tick: u64,
    elapsed_ms: i96,
};

const Timer = struct {
    interval_ns: i96,
    started_ns: i96,
    deadline_ns: i96,
    completion: []u8,
    tick: u64 = 0,
};

pub const Collection = struct {
    allocator: std.mem.Allocator,
    timers: std.StringHashMap(Timer),

    pub fn init(allocator: std.mem.Allocator) Collection {
        return .{ .allocator = allocator, .timers = .init(allocator) };
    }

    pub fn deinit(self: *Collection) void {
        var timers = self.timers.iterator();
        while (timers.next()) |entry| {
            self.allocator.free(entry.key_ptr.*);
            self.allocator.free(entry.value_ptr.completion);
        }
        self.timers.deinit();
    }

    pub fn count(self: *const Collection) u32 {
        return self.timers.count();
    }

    pub fn nextDeadline(self: *const Collection) ?i96 {
        var result: ?i96 = null;
        var iterator = self.timers.iterator();
        while (iterator.next()) |entry| {
            if (result == null or entry.value_ptr.deadline_ns < result.?)
                result = entry.value_ptr.deadline_ns;
        }
        return result;
    }

    pub fn start(self: *Collection, spec: Start, now_ns: i96) !void {
        self.stop(spec.id);
        const completion = try self.allocator.dupe(u8, spec.completion);
        errdefer self.allocator.free(completion);
        const id = try self.allocator.dupe(u8, spec.id);
        errdefer self.allocator.free(id);
        const interval_ns: i96 = @as(i96, @intCast(spec.interval_ms)) * std.time.ns_per_ms;
        try self.timers.put(id, .{
            .interval_ns = interval_ns,
            .started_ns = now_ns,
            .deadline_ns = now_ns + interval_ns,
            .completion = completion,
        });
    }

    pub fn stop(self: *Collection, id: []const u8) void {
        const removed = self.timers.fetchRemove(id) orelse return;
        self.allocator.free(removed.key);
        self.allocator.free(removed.value.completion);
    }

    pub fn due(self: *Collection, now_ns: i96) DueIterator {
        return .{ .iterator = self.timers.iterator(), .now_ns = now_ns };
    }
};

pub const DueIterator = struct {
    iterator: std.StringHashMap(Timer).Iterator,
    now_ns: i96,

    pub fn next(self: *DueIterator) ?Tick {
        while (self.iterator.next()) |entry| {
            const item = entry.value_ptr;
            if (self.now_ns < item.deadline_ns) continue;
            item.tick +|= 1;
            // Advance from the prior deadline to avoid drift, but coalesce
            // missed periods so a delayed consumer is never flooded.
            item.deadline_ns += item.interval_ns;
            if (item.deadline_ns <= self.now_ns) item.deadline_ns = self.now_ns + item.interval_ns;
            return .{
                .completion = item.completion,
                .id = entry.key_ptr.*,
                .tick = item.tick,
                .elapsed_ms = @divTrunc(self.now_ns - item.started_ns, std.time.ns_per_ms),
            };
        }
        return null;
    }
};

test "registrations compose by id and replacement preserves other timers" {
    var timers: Collection = .init(std.testing.allocator);
    defer timers.deinit();

    try timers.start(.{ .interval_ms = 20, .completion = "first/tick", .id = "first" }, 0);
    try timers.start(.{ .interval_ms = 30, .completion = "second/tick", .id = "second" }, 0);
    try std.testing.expectEqual(@as(u32, 2), timers.count());
    try timers.start(.{ .interval_ms = 40, .completion = "first/replaced", .id = "first" }, 0);
    try std.testing.expectEqual(@as(u32, 2), timers.count());

    var due = timers.due(35 * std.time.ns_per_ms);
    const tick = due.next().?;
    try std.testing.expectEqualStrings("second", tick.id);
    try std.testing.expectEqualStrings("second/tick", tick.completion);
    try std.testing.expectEqual(@as(u64, 1), tick.tick);
    try std.testing.expectEqual(@as(i96, 35), tick.elapsed_ms);
    try std.testing.expect(due.next() == null);

    timers.stop("first");
    try std.testing.expectEqual(@as(u32, 1), timers.count());
}
