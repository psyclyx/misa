//! Bounded record channel with level-triggered POSIX wakeup notification.
const std = @import("std");
const posix = std.posix;

pub const Record = union(enum) { data: []u8, event: []u8, terminal };

pub const Drain = struct {
    count: usize = 0,
    event: ?[]u8 = null,
    terminal: bool = false,
};

pub const Wakeup = struct {
    read_fd: posix.fd_t,
    write_fd: posix.fd_t,

    pub fn init() !Wakeup {
        var fds: [2]posix.fd_t = undefined;
        switch (posix.errno(posix.system.pipe2(&fds, .{ .NONBLOCK = true, .CLOEXEC = true }))) {
            .SUCCESS => {},
            else => return error.WakeupPipeUnavailable,
        }
        return .{ .read_fd = fds[0], .write_fd = fds[1] };
    }

    pub fn deinit(self: Wakeup) void {
        _ = posix.system.close(self.read_fd);
        _ = posix.system.close(self.write_fd);
    }

    /// Drain the level-triggered wakeup. Writers may coalesce notifications;
    /// channel and completion state remain authoritative.
    pub fn consume(self: Wakeup) void {
        var bytes: [128]u8 = undefined;
        while (true) switch (posix.errno(posix.system.read(self.read_fd, &bytes, bytes.len))) {
            .SUCCESS => continue,
            .AGAIN => return,
            .INTR => continue,
            else => return,
        };
    }

    pub fn notify(self: Wakeup) void {
        const byte: [1]u8 = .{1};
        while (true) switch (posix.errno(posix.system.write(self.write_fd, &byte, byte.len))) {
            .SUCCESS, .AGAIN => return,
            .INTR => continue,
            else => return,
        };
    }
};

pub const Channel = struct {
    pub const capacity = 32;

    allocator: std.mem.Allocator,
    io: std.Io,
    wakeup: Wakeup,
    mutex: std.Io.Mutex = .init,
    not_full: std.Io.Condition = .init,
    items: [capacity]?Record = .{null} ** capacity,
    head: usize = 0,
    len: usize = 0,
    activity_ns: std.atomic.Value(i64) = .init(0),
    saw_activity: std.atomic.Value(bool) = .init(false),

    pub fn init(allocator: std.mem.Allocator, io: std.Io, wakeup: Wakeup) Channel {
        return .{ .allocator = allocator, .io = io, .wakeup = wakeup };
    }

    pub fn resetActivity(self: *Channel, now_ns: i96) void {
        self.saw_activity.store(false, .release);
        self.activity_ns.store(@intCast(now_ns), .release);
    }

    pub fn noteActivity(self: *Channel) void {
        self.saw_activity.store(true, .release);
        self.activity_ns.store(@intCast(std.Io.Timestamp.now(self.io, .awake).nanoseconds), .release);
    }

    pub fn push(self: *Channel, record: Record) !void {
        {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            while (self.len == capacity) try self.not_full.wait(self.io, &self.mutex);
            self.items[(self.head + self.len) % capacity] = record;
            self.len += 1;
            self.noteActivity();
        }
        self.wakeup.notify();
    }

    pub fn pop(self: *Channel) ?Record {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.len == 0) return null;
        const record = self.take();
        self.not_full.signal(self.io);
        return record;
    }

    /// Drain one publication batch while excluding the producer. Events and
    /// terminal markers end a batch; data records are transferred to `output`.
    pub fn drain(self: *Channel, output: [][]u8) Drain {
        var result: Drain = .{};
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        while (self.len != 0 and result.count < output.len) {
            switch (self.take()) {
                .data => |json| {
                    output[result.count] = json;
                    result.count += 1;
                },
                .event => |json| {
                    result.event = json;
                    self.not_full.signal(self.io);
                    return result;
                },
                .terminal => result.terminal = true,
            }
            if (result.terminal) break;
        }
        if (result.count != 0 or result.terminal) self.not_full.signal(self.io);
        return result;
    }

    pub fn count(self: *Channel) usize {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        return self.len;
    }

    pub fn discard(self: *Channel) void {
        while (self.pop()) |record| switch (record) {
            .data, .event => |json| self.allocator.free(json),
            .terminal => {},
        };
    }

    fn take(self: *Channel) Record {
        std.debug.assert(self.len != 0);
        const record = self.items[self.head].?;
        self.items[self.head] = null;
        self.head = (self.head + 1) % capacity;
        self.len -= 1;
        return record;
    }
};

test "bounded channel preserves records and signals its wakeup" {
    const wakeup = try Wakeup.init();
    defer wakeup.deinit();
    var channel: Channel = .init(std.testing.allocator, std.testing.io, wakeup);
    channel.resetActivity(std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds);

    try channel.push(.terminal);
    try std.testing.expectEqual(@as(usize, 1), channel.count());
    var fds = [_]posix.pollfd{.{ .fd = wakeup.read_fd, .events = posix.POLL.IN, .revents = 0 }};
    try std.testing.expect((try posix.poll(&fds, 0)) == 1);
    wakeup.consume();
    fds[0].revents = 0;
    try std.testing.expect((try posix.poll(&fds, 0)) == 0);
    try std.testing.expect(channel.pop().? == .terminal);
}
