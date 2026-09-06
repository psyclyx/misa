//! Terminal actor: one owner of tty lifetime, decoding, and presentation state.
const std = @import("std");
const terminal_module = @import("root.zig");
const Wakeup = @import("misa_wakeup").Wakeup;
const Driver = @This();

pub const View = struct {
    arena: std.heap.ArenaAllocator,
    value: std.json.Value,
    dimensions: terminal_module.Dimensions,
    pub fn deinit(self: *View) void {
        self.arena.deinit();
    }
};
pub const Input = union(enum) {
    key: terminal_module.Event,
    action: []u8,
    pub fn deinit(self: Input, allocator: std.mem.Allocator) void {
        switch (self) {
            .action => |value| allocator.free(value),
            .key => |event| {
                switch (event) {
                    .text, .alt => |bytes| std.crypto.secureZero(u8, bytes),
                    else => {},
                }
                event.deinit(allocator);
            },
        }
    }
};
pub const Info = struct { interactive: bool, images_supported: bool, dimensions: terminal_module.Dimensions };
const Command = union(enum) {
    flush,
    clipboard: []const u8,
    commit: std.json.Value,
    acquire,
    release: terminal_module.Terminal.HandoffLease,
    discard,
};
const capacity = 64;
const frame_ns = 16 * std.time.ns_per_ms;
allocator: std.mem.Allocator,
io: std.Io,
terminal: terminal_module.Terminal,
thread: ?std.Thread = null,
actor_wakeup: Wakeup,
session_wakeup: Wakeup,
mutex: std.Io.Mutex = .init,
ack: std.Io.Condition = .init,
stopping: bool = false,
failure: ?anyerror = null,
metadata: Info,
resize: ?terminal_module.Dimensions = null,
pending: ?View = null,
input_enabled: bool = false,
inputs: [capacity]?Input = .{null} ** capacity,
head: usize = 0,
len: usize = 0,
command: ?Command = null,
command_done: bool = false,
command_error: ?anyerror = null,
lease: ?terminal_module.Terminal.HandoffLease = null,
// Everything below is exclusively accessed by the actor until it joins.
decoded: std.ArrayList(terminal_module.Event) = .empty,
decoded_head: usize = 0,
decoded_actions: @import("hit_map.zig").Map = .{},
decoder_deadline: ?i96 = null,
eof: bool = false,
last_frame: i96 = 0,

pub fn create(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) !*Driver {
    const self = try allocator.create(Driver);
    errdefer allocator.destroy(self);
    var tty = try terminal_module.Terminal.init(allocator, io, environ);
    errdefer tty.deinit();
    const actor_wakeup = try Wakeup.init();
    errdefer actor_wakeup.deinit();
    const session_wakeup = try Wakeup.init();
    errdefer session_wakeup.deinit();
    self.* = .{
        .allocator = allocator,
        .io = io,
        .terminal = tty,
        .actor_wakeup = actor_wakeup,
        .session_wakeup = session_wakeup,
        .metadata = .{ .interactive = tty.interactive, .images_supported = tty.images_supported, .dimensions = tty.dimensions },
    };
    self.thread = try std.Thread.spawn(.{}, worker, .{self});
    return self;
}
pub fn destroy(self: *Driver) void {
    self.mutex.lockUncancelable(self.io);
    self.stopping = true;
    self.mutex.unlock(self.io);
    self.actor_wakeup.notify();
    self.thread.?.join();
    if (self.pending) |*view| view.deinit();
    self.clearInput();
    self.decoded.deinit(self.allocator);
    self.decoded_actions.deinit(self.allocator);
    self.terminal.deinit();
    self.actor_wakeup.deinit();
    self.session_wakeup.deinit();
    self.allocator.destroy(self);
}
pub fn info(self: *Driver) Info {
    self.mutex.lockUncancelable(self.io);
    defer self.mutex.unlock(self.io);
    return self.metadata;
}
pub fn checkError(self: *Driver) !void {
    self.mutex.lockUncancelable(self.io);
    defer self.mutex.unlock(self.io);
    if (self.failure) |err| return err;
}
pub fn wakeupFd(self: *const Driver) std.posix.fd_t {
    return self.session_wakeup.read_fd;
}
pub fn consumeWakeup(self: *Driver) void {
    self.session_wakeup.consume();
}
pub fn publish(self: *Driver, view: View) !void {
    self.mutex.lockUncancelable(self.io);
    if (self.failure) |err| {
        self.mutex.unlock(self.io);
        return err;
    }
    const old = self.pending;
    self.pending = view;
    self.mutex.unlock(self.io);
    if (old) |value| {
        var dropped = value;
        dropped.deinit();
    }
    self.actor_wakeup.notify();
}
pub fn enableInput(self: *Driver) !void {
    self.mutex.lockUncancelable(self.io);
    defer self.mutex.unlock(self.io);
    if (self.failure) |err| return err;
    self.input_enabled = true;
    self.actor_wakeup.notify();
}
pub fn popInput(self: *Driver) !?Input {
    self.mutex.lockUncancelable(self.io);
    defer self.mutex.unlock(self.io);
    if (self.failure) |err| return err;
    if (self.len == 0) return null;
    const value = self.inputs[self.head].?;
    self.inputs[self.head] = null;
    self.head = (self.head + 1) % capacity;
    self.len -= 1;
    if (self.len != 0) self.session_wakeup.notify();
    self.actor_wakeup.notify();
    return value;
}
pub fn takeResize(self: *Driver) !?terminal_module.Dimensions {
    self.mutex.lockUncancelable(self.io);
    defer self.mutex.unlock(self.io);
    if (self.failure) |err| return err;
    const value = self.resize;
    self.resize = null;
    return value;
}
pub fn flush(self: *Driver) !void {
    try self.rpc(.flush);
}
pub fn copyToClipboard(self: *Driver, value: []const u8) !void {
    try self.rpc(.{ .clipboard = value });
}
pub fn commit(self: *Driver, value: std.json.Value) !void {
    try self.rpc(.{ .commit = value });
}
pub fn acquireHandoff(self: *Driver) !terminal_module.Terminal.HandoffLease {
    try self.rpc(.acquire);
    return self.lease.?;
}
pub fn releaseHandoff(self: *Driver, value: terminal_module.Terminal.HandoffLease) !void {
    try self.rpc(.{ .release = value });
}
pub fn discardInput(self: *Driver) !void {
    try self.rpc(.discard);
}

// Session is the sole RPC producer. Borrowed arguments remain valid until ack.
fn rpc(self: *Driver, value: Command) !void {
    self.mutex.lockUncancelable(self.io);
    defer self.mutex.unlock(self.io);
    if (self.failure) |err| return err;
    std.debug.assert(self.command == null);
    self.command_done = false;
    self.command_error = null;
    self.command = value;
    self.actor_wakeup.notify();
    while (!self.command_done and self.failure == null) self.ack.waitUncancelable(self.io, &self.mutex);
    if (self.failure) |err| return err;
    if (self.command_error) |err| return err;
}
fn worker(self: *Driver) void {
    self.loop() catch |err| {
        self.mutex.lockUncancelable(self.io);
        self.failure = err;
        self.ack.broadcast(self.io);
        self.mutex.unlock(self.io);
        self.session_wakeup.notify();
    };
}
fn loop(self: *Driver) !void {
    while (true) {
        self.actor_wakeup.consume();
        self.mutex.lockUncancelable(self.io);
        const stop = self.stopping;
        const command = self.command;
        self.mutex.unlock(self.io);
        if (stop) return;
        if (command) |value| {
            const err: ?anyerror = if (self.execute(value)) |_| null else |failure| failure;
            self.mutex.lockUncancelable(self.io);
            self.command = null;
            self.command_error = err;
            self.command_done = true;
            self.ack.signal(self.io);
            self.mutex.unlock(self.io);
        }
        if (try self.terminal.pollResize()) {
            self.mutex.lockUncancelable(self.io);
            self.metadata.dimensions = self.terminal.dimensions;
            self.resize = self.terminal.dimensions;
            self.mutex.unlock(self.io);
            self.session_wakeup.notify();
        }
        try self.render(false);
        try self.drainDecoded();
        const now = std.Io.Timestamp.now(self.io, .awake).nanoseconds;
        if (self.decoder_deadline) |deadline| if (now >= deadline) {
            try self.captureActions();
            try self.terminal.resolveDecoderTimeout(&self.decoded);
            self.decoder_deadline = null;
            try self.drainDecoded();
        };
        self.mutex.lockUncancelable(self.io);
        const want_input = self.input_enabled and self.len < capacity and self.decoded_head == self.decoded.items.len and !self.eof;
        const pending = self.pending != null;
        self.mutex.unlock(self.io);
        var timeout: i32 = 100;
        if (pending and self.terminal.presentationEnabled()) timeout = @intCast(@min(timeout, @max(0, @divFloor(self.last_frame + frame_ns - now + std.time.ns_per_ms - 1, std.time.ns_per_ms))));
        if (self.terminal.nextAnimationDeadline()) |deadline| {
            const due = @max(deadline, self.last_frame + frame_ns);
            timeout = @intCast(@min(timeout, @max(0, @divFloor(due - now + std.time.ns_per_ms - 1, std.time.ns_per_ms))));
        }
        if (self.decoder_deadline) |deadline| timeout = @intCast(@min(timeout, @max(0, @divFloor(deadline - now + std.time.ns_per_ms - 1, std.time.ns_per_ms))));
        const ready = try self.terminal.waitSources(self.actor_wakeup.read_fd, want_input, timeout);
        // Commands win readiness races with stdin (especially handoff/discard).
        if (ready.operation) continue;
        if (ready.input) {
            self.decoded.clearRetainingCapacity();
            self.decoded_head = 0;
            try self.captureActions();
            try self.terminal.readEvents(&self.decoded);
            self.decoder_deadline = if (self.terminal.decoderNeedsTimeout()) std.Io.Timestamp.now(self.io, .awake).nanoseconds + 25 * std.time.ns_per_ms else null;
            try self.drainDecoded();
        }
    }
}
fn execute(self: *Driver, value: Command) !void {
    switch (value) {
        .flush => try self.render(true),
        .clipboard => |bytes| try self.terminal.copyToClipboard(bytes),
        .commit => |lines| {
            try self.render(true);
            try self.terminal.commit(lines);
        },
        .acquire => {
            if (self.terminal.active_handoff != null) return error.TerminalHandoffBusy;
            try self.render(true);
            self.clearInput();
            self.terminal.discardInput();
            self.lease = try self.terminal.acquireHandoff();
        },
        .release => |lease| try self.terminal.releaseHandoff(lease),
        .discard => {
            self.clearInput();
            self.terminal.discardInput();
        },
    }
}
fn render(self: *Driver, force: bool) !void {
    if (!self.terminal.presentationEnabled()) return;
    const now = std.Io.Timestamp.now(self.io, .awake).nanoseconds;
    if (!force and self.last_frame != 0 and now - self.last_frame < frame_ns) return;
    self.mutex.lockUncancelable(self.io);
    var view = self.pending;
    self.pending = null;
    self.mutex.unlock(self.io);
    if (view) |*owned| {
        defer owned.deinit();
        if (owned.dimensions.columns != self.terminal.dimensions.columns or owned.dimensions.lines != self.terminal.dimensions.lines) return;
        var prepared = try self.terminal.preparePresentation(owned.value);
        defer prepared.deinit(self.allocator);
        try self.terminal.present(&prepared);
        self.last_frame = std.Io.Timestamp.now(self.io, .awake).nanoseconds;
    } else if (try self.terminal.advanceAnimations(now)) {
        self.last_frame = std.Io.Timestamp.now(self.io, .awake).nanoseconds;
    }
}
fn clearInput(self: *Driver) void {
    self.mutex.lockUncancelable(self.io);
    while (self.len != 0) {
        self.inputs[self.head].?.deinit(self.allocator);
        self.inputs[self.head] = null;
        self.head = (self.head + 1) % capacity;
        self.len -= 1;
    }
    self.mutex.unlock(self.io);
    for (self.decoded.items[self.decoded_head..]) |event| (Input{ .key = event }).deinit(self.allocator);
    self.decoded.clearRetainingCapacity();
    self.decoded_head = 0;
    self.decoder_deadline = null;
    // A discarded EOF must be observable again; otherwise the session could
    // wait forever after cancellation clears the queued EOF notification.
    self.eof = false;
}
fn drainDecoded(self: *Driver) !void {
    while (self.decoded_head < self.decoded.items.len) {
        self.mutex.lockUncancelable(self.io);
        const full = self.len == capacity;
        self.mutex.unlock(self.io);
        if (full) return;
        const event = self.decoded.items[self.decoded_head];
        var value: Input = .{ .key = event };
        if (event == .mouse) if (self.decoded_actions.at(event.mouse.row, event.mouse.column)) |action| {
            value = .{ .action = try self.allocator.dupe(u8, action) };
        };
        self.decoded_head += 1;
        if (event == .eof) self.eof = true;
        self.mutex.lockUncancelable(self.io);
        self.inputs[(self.head + self.len) % capacity] = value;
        self.len += 1;
        self.mutex.unlock(self.io);
        self.session_wakeup.notify();
    }
    self.decoded.clearRetainingCapacity();
    self.decoded_head = 0;
}

// A batch may outlive a frame while the bounded ring is full. Preserve the
// displayed hit map from decoding time for its remaining mouse events.
fn captureActions(self: *Driver) !void {
    if (self.decoded_head != self.decoded.items.len) return;
    self.decoded_actions.deinit(self.allocator);
    if (self.terminal.screen_active) self.decoded_actions = try self.terminal.presented_actions.clone(self.allocator);
}

test "input ring remains bounded and actor services discard and flush while full" {
    const allocator = std.testing.allocator;
    const io = std.testing.io;
    const actor_wakeup = try Wakeup.init();
    defer actor_wakeup.deinit();
    const session_wakeup = try Wakeup.init();
    defer session_wakeup.deinit();
    var driver: Driver = .{
        .allocator = allocator,
        .io = io,
        .terminal = .{ .allocator = allocator, .io = io, .interactive = false, .dimensions = .{} },
        .actor_wakeup = actor_wakeup,
        .session_wakeup = session_wakeup,
        .metadata = .{ .interactive = false, .images_supported = false, .dimensions = .{} },
    };
    defer driver.terminal.deinit();
    defer driver.decoded.deinit(allocator);
    defer driver.decoded_actions.deinit(allocator);
    defer driver.clearInput();
    for (0..80) |_| try driver.decoded.append(allocator, .enter);
    try driver.drainDecoded();
    try std.testing.expectEqual(capacity, driver.len);
    try std.testing.expectEqual(@as(usize, 64), driver.decoded_head);
    driver.consumeWakeup();
    const first = (try driver.popInput()).?;
    var fds = [_]std.posix.pollfd{.{ .fd = driver.wakeupFd(), .events = std.posix.POLL.IN, .revents = 0 }};
    try std.testing.expectEqual(@as(usize, 1), try std.posix.poll(&fds, 0));
    defer first.deinit(allocator);
    try std.testing.expect(first.key == .enter);
    try driver.drainDecoded();
    try std.testing.expectEqual(capacity, driver.len);
    driver.eof = true;
    driver.thread = try std.Thread.spawn(.{}, worker, .{&driver});
    defer {
        driver.mutex.lockUncancelable(io);
        driver.stopping = true;
        driver.mutex.unlock(io);
        driver.actor_wakeup.notify();
        driver.thread.?.join();
    }
    try driver.flush();
    try driver.discardInput();
    try std.testing.expect(!driver.eof);
    const lease = try driver.acquireHandoff();
    try std.testing.expectError(error.TerminalHandoffBusy, driver.acquireHandoff());
    try driver.releaseHandoff(lease);
    try std.testing.expect((try driver.popInput()) == null);
    try driver.checkError();
}

test "latest view mailbox owns replacements and preserves ownership on failure" {
    const allocator = std.testing.allocator;
    const io = std.testing.io;
    const actor_wakeup = try Wakeup.init();
    defer actor_wakeup.deinit();
    const session_wakeup = try Wakeup.init();
    defer session_wakeup.deinit();
    var driver: Driver = .{
        .allocator = allocator,
        .io = io,
        .terminal = .{ .allocator = allocator, .io = io, .interactive = false, .dimensions = .{} },
        .actor_wakeup = actor_wakeup,
        .session_wakeup = session_wakeup,
        .metadata = .{ .interactive = false, .images_supported = false, .dimensions = .{} },
    };
    defer if (driver.pending) |*view| view.deinit();
    for (0..3) |i| {
        var arena = std.heap.ArenaAllocator.init(allocator);
        errdefer arena.deinit();
        _ = try arena.allocator().alloc(u8, 123);
        try driver.publish(.{ .arena = arena, .value = .{ .integer = @intCast(i) }, .dimensions = .{} });
    }
    try std.testing.expectEqual(@as(i64, 2), driver.pending.?.value.integer);
    driver.failure = error.TestDriverFailed;
    var rejected = View{ .arena = std.heap.ArenaAllocator.init(allocator), .value = .null, .dimensions = .{} };
    defer rejected.deinit();
    _ = try rejected.arena.allocator().alloc(u8, 100);
    try std.testing.expectError(error.TestDriverFailed, driver.publish(rejected));
}
