//! Portable terminal lifetime, decoding, signal restoration, and managed-screen presentation.
const std = @import("std");
const posix = std.posix;
const input = @import("input.zig");
const presenter = @import("presenter.zig");

pub const Event = input.Event;
pub const Decoder = input.Decoder;
pub const validateLines = presenter.validateLines;
const appendScreenPrelude = presenter.appendScreenPrelude;
const appendView = presenter.appendView;
const appendLines = presenter.appendLines;
const cursorRow = presenter.cursorRow;
pub const usableColumns = presenter.usableColumns;

pub const Dimensions = struct { columns: usize = 80, lines: usize = 24 };

pub const PreparedPresentation = struct {
    bytes: std.ArrayList(u8) = .empty,

    pub fn deinit(self: *PreparedPresentation, allocator: std.mem.Allocator) void {
        self.bytes.deinit(allocator);
    }
};

const show_cursor = "\x1b[?25h";
const enter_managed_screen = "\x1b[?25l\x1b[?1049h\x1b[H\x1b[2J";
const leave_managed_screen = "\x1b[?25h\x1b[?1049l";

const handled_signals = [_]posix.SIG{ .HUP, .INT, .QUIT, .TERM };
const SignalState = struct {
    active: std.atomic.Value(bool) = .init(false),
    screen_active: std.atomic.Value(bool) = .init(false),
    resize_pending: std.atomic.Value(bool) = .init(false),
    saved_termios: posix.termios = undefined,
    old_actions: [handled_signals.len]posix.Sigaction = undefined,
    old_winch: posix.Sigaction = undefined,
};
var signal_state: SignalState = .{};

fn restoreOnSignal(sig: posix.SIG) callconv(.c) void {
    if (!signal_state.active.load(.acquire)) return;
    if (signal_state.screen_active.load(.acquire))
        _ = posix.system.write(posix.STDOUT_FILENO, leave_managed_screen.ptr, leave_managed_screen.len)
    else
        _ = posix.system.write(posix.STDOUT_FILENO, show_cursor.ptr, show_cursor.len);
    posix.tcsetattr(posix.STDIN_FILENO, .NOW, signal_state.saved_termios) catch {};
    for (handled_signals, 0..) |candidate, i| {
        if (candidate == sig) {
            posix.sigaction(candidate, &signal_state.old_actions[i], null);
            posix.raise(candidate) catch {};
            return;
        }
    }
}

fn noteResize(_: posix.SIG) callconv(.c) void {
    signal_state.resize_pending.store(true, .release);
}

fn rawMode(original: posix.termios) posix.termios {
    var raw = original;
    raw.lflag.ICANON = false;
    raw.lflag.ECHO = false;
    raw.lflag.IEXTEN = false;
    raw.lflag.ISIG = false;
    raw.iflag.IXON = false;
    raw.iflag.ICRNL = false;
    raw.iflag.BRKINT = false;
    raw.iflag.INPCK = false;
    raw.iflag.ISTRIP = false;
    // Reads are issued only after poll(2) reports readiness. Keep the fd
    // genuinely nonblocking at the tty layer as well: no hidden VTIME sleep.
    raw.cc[@intFromEnum(posix.V.MIN)] = 0;
    raw.cc[@intFromEnum(posix.V.TIME)] = 0;
    return raw;
}

pub const Terminal = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    interactive: bool,
    dimensions: Dimensions,
    saved: ?posix.termios = null,
    signals_installed: bool = false,
    screen_active: bool = false,
    handoff_generation: u64 = 0,
    active_handoff: ?u64 = null,
    decoder: Decoder = .{},
    last_frame: std.ArrayList(u8) = .empty,

    pub const HandoffLease = struct { generation: u64 };

    pub fn init(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) !Terminal {
        const term = environ.get("TERM") orelse "";
        // Use the stdlib's actual tty query for classification. tcgetattr on
        // stdout is not a portable isatty substitute and fails in some real
        // terminal/build-runner combinations even though the fd is a tty.
        const input_tty = std.Io.File.stdin().isTty(io) catch false;
        const output_tty = std.Io.File.stdout().isTty(io) catch false;
        const input_attr = if (input_tty) posix.tcgetattr(posix.STDIN_FILENO) catch null else null;
        const interactive = input_attr != null and output_tty and term.len != 0 and !std.mem.eql(u8, term, "dumb");
        var self: Terminal = .{
            .allocator = allocator,
            .io = io,
            .interactive = interactive,
            .dimensions = if (output_tty) queryDimensions(io) catch dimensions(environ) else dimensions(environ),
        };
        if (interactive) {
            std.debug.assert(!signal_state.active.load(.acquire));
            self.saved = input_attr.?;
            signal_state.saved_termios = input_attr.?;
            var signal_mask = posix.sigemptyset();
            for (handled_signals) |sig| posix.sigaddset(&signal_mask, sig);
            posix.sigaddset(&signal_mask, .WINCH);
            var old_mask: posix.sigset_t = undefined;
            posix.sigprocmask(posix.SIG.BLOCK, &signal_mask, &old_mask);
            defer posix.sigprocmask(posix.SIG.SETMASK, &old_mask, null);
            const action: posix.Sigaction = .{ .handler = .{ .handler = restoreOnSignal }, .mask = posix.sigemptyset(), .flags = 0 };
            for (handled_signals, 0..) |sig, i| posix.sigaction(sig, &action, &signal_state.old_actions[i]);
            const resize_action: posix.Sigaction = .{ .handler = .{ .handler = noteResize }, .mask = posix.sigemptyset(), .flags = 0 };
            posix.sigaction(.WINCH, &resize_action, &signal_state.old_winch);
            signal_state.resize_pending.store(false, .release);
            signal_state.active.store(true, .release);
            self.signals_installed = true;
            posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, rawMode(input_attr.?)) catch |err| {
                self.restoreSignals();
                return err;
            };
            self.screen_active = true;
            signal_state.screen_active.store(true, .release);
            self.writeAll(enter_managed_screen) catch |err| {
                self.deinit();
                return err;
            };
        }
        return self;
    }

    /// Exclusively transfer stdin to an external command. A generation token
    /// prevents a stale completion from resuming over a newer owner.
    pub fn acquireHandoff(self: *Terminal) !HandoffLease {
        if (self.active_handoff != null) return error.TerminalHandoffBusy;
        self.handoff_generation +%= 1;
        if (self.handoff_generation == 0) self.handoff_generation = 1;
        const lease: HandoffLease = .{ .generation = self.handoff_generation };
        self.active_handoff = lease.generation;
        errdefer {
            // A partial suspend must not strand either the generation lease or
            // terminal ownership. Recovery is best-effort; the original error
            // still reports that no handoff was granted.
            self.active_handoff = null;
            if (self.saved) |saved| posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, rawMode(saved)) catch {};
            if (self.interactive and !self.screen_active) {
                if (self.writeAll(enter_managed_screen)) |_| {
                    self.screen_active = true;
                    signal_state.screen_active.store(true, .release);
                } else |_| {}
            }
        }
        if (self.screen_active) {
            try self.writeAll(leave_managed_screen);
            self.screen_active = false;
            signal_state.screen_active.store(false, .release);
        }
        if (self.interactive) try self.writeAll(show_cursor);
        if (self.saved) |saved| try posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, saved);
        return lease;
    }

    pub fn releaseHandoff(self: *Terminal, lease: HandoffLease) !void {
        if (self.active_handoff == null or self.active_handoff.? != lease.generation)
            return error.StaleTerminalHandoff;
        // Consume the matching generation regardless of resume I/O outcome. A
        // failed resume is a clean failed operation, never a permanent busy
        // lease that wedges every future auth command.
        self.active_handoff = null;
        if (self.saved) |saved| posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, rawMode(saved)) catch |err| {
            posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, saved) catch {};
            return err;
        };
        if (self.interactive and !self.screen_active) {
            self.writeAll(enter_managed_screen) catch |err| {
                if (self.saved) |saved| posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, saved) catch {};
                return err;
            };
            self.screen_active = true;
            signal_state.screen_active.store(true, .release);
            // Alternate-screen contents are unspecified after resume.
            if (self.last_frame.items.len != 0) self.writeAll(self.last_frame.items) catch |err| {
                self.writeAll(leave_managed_screen) catch {};
                self.screen_active = false;
                signal_state.screen_active.store(false, .release);
                if (self.saved) |saved| posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, saved) catch {};
                return err;
            };
        }
    }

    pub fn presentationEnabled(self: *const Terminal) bool {
        return !self.interactive or self.screen_active;
    }

    pub fn deinit(self: *Terminal) void {
        if (self.interactive) {
            if (self.screen_active) self.writeAll(leave_managed_screen) catch {} else self.writeAll(show_cursor) catch {};
            self.screen_active = false;
            signal_state.screen_active.store(false, .release);
            if (self.saved) |saved| posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, saved) catch {};
            self.restoreSignals();
        }
        self.decoder.deinit(self.allocator);
        self.last_frame.deinit(self.allocator);
    }

    fn restoreSignals(self: *Terminal) void {
        if (!self.signals_installed) return;
        var signal_mask = posix.sigemptyset();
        for (handled_signals) |sig| posix.sigaddset(&signal_mask, sig);
        posix.sigaddset(&signal_mask, .WINCH);
        var old_mask: posix.sigset_t = undefined;
        posix.sigprocmask(posix.SIG.BLOCK, &signal_mask, &old_mask);
        signal_state.active.store(false, .release);
        for (handled_signals, 0..) |sig, i| posix.sigaction(sig, &signal_state.old_actions[i], null);
        posix.sigaction(.WINCH, &signal_state.old_winch, null);
        self.signals_installed = false;
        posix.sigprocmask(posix.SIG.SETMASK, &old_mask, null);
    }

    /// Consume SIGWINCH's atomic notification and refresh dimensions in normal
    /// execution context. Returns true only for a real dimension change.
    pub fn pollResize(self: *Terminal) !bool {
        if (!self.interactive or !signal_state.resize_pending.swap(false, .acq_rel)) return false;
        const updated = try queryDimensions(self.io);
        if (updated.columns == self.dimensions.columns and updated.lines == self.dimensions.lines) return false;
        self.dimensions = updated;
        return true;
    }

    /// Validate and render once. The session can then write the exact prepared
    /// bytes before committing Lua state without paying for a second render.
    pub fn preparePresentation(self: *const Terminal, view: std.json.Value) !PreparedPresentation {
        var prepared: PreparedPresentation = .{};
        errdefer prepared.deinit(self.allocator);
        if (self.interactive) try appendScreenPrelude(&prepared.bytes, self.allocator);
        const rows = try appendView(&prepared.bytes, self.allocator, view, self.dimensions.columns, self.dimensions.lines, self.interactive, false);
        _ = try cursorRow(view, rows);
        return prepared;
    }

    pub fn present(self: *Terminal, prepared: *const PreparedPresentation) !void {
        if (!self.interactive) return;
        if (!self.screen_active) return error.TerminalSuspended;
        try self.writeAll(prepared.bytes.items);
        self.last_frame.clearRetainingCapacity();
        try self.last_frame.appendSlice(self.allocator, prepared.bytes.items);
    }

    pub fn commit(self: *Terminal, lines: std.json.Value) !void {
        var buffer: std.ArrayList(u8) = .empty;
        defer buffer.deinit(self.allocator);
        // Build and validate everything before touching terminal ownership.
        if (self.interactive) try appendScreenPrelude(&buffer, self.allocator);
        const rows = try appendLines(&buffer, self.allocator, lines, self.dimensions.columns, self.interactive, !self.interactive, false);
        if (self.interactive and rows > self.dimensions.lines) return error.InvalidView;
        try self.writeAll(buffer.items);
    }

    /// Readiness probe used while native work is active, so the session never
    /// enters even the tty's short VTIME wait on its owner thread.
    pub fn inputReady(self: *const Terminal) !bool {
        _ = self;
        var fds = [_]posix.pollfd{.{ .fd = posix.STDIN_FILENO, .events = posix.POLL.IN, .revents = 0 }};
        return try posix.poll(&fds, 0) != 0;
    }

    pub const ReadySources = struct { input: bool = false, operation: bool = false };

    /// Wait once for every asynchronous source owned by the session. The
    /// operation pipe is always present; stdin is omitted during handoff.
    pub fn waitSources(self: *const Terminal, operation_fd: posix.fd_t, want_input: bool, timeout_ms: i32) !ReadySources {
        var fds = [_]posix.pollfd{
            .{ .fd = posix.STDIN_FILENO, .events = posix.POLL.IN, .revents = 0 },
            .{ .fd = operation_fd, .events = posix.POLL.IN, .revents = 0 },
        };
        if (!want_input or self.active_handoff != null) fds[0].fd = -1;
        _ = try posix.poll(&fds, timeout_ms);
        return .{
            .input = fds[0].fd >= 0 and (fds[0].revents & (posix.POLL.IN | posix.POLL.HUP | posix.POLL.ERR)) != 0,
            .operation = (fds[1].revents & (posix.POLL.IN | posix.POLL.HUP | posix.POLL.ERR)) != 0,
        };
    }

    pub fn decoderNeedsTimeout(self: *const Terminal) bool {
        return self.decoder.needsTimeout();
    }

    pub fn resolveDecoderTimeout(self: *Terminal, out: *std.ArrayList(Event)) !void {
        try self.decoder.finish(self.allocator, out);
    }

    pub fn readEvents(self: *Terminal, out: *std.ArrayList(Event)) !void {
        var bytes: [4096]u8 = undefined;
        const n = std.Io.File.stdin().readStreaming(self.io, &.{&bytes}) catch |err| switch (err) {
            error.EndOfStream => {
                // Interactive reads are readiness-gated with VMIN=0/VTIME=0.
                // A zero-byte result is a transient empty read, not tty EOF.
                switch (classifyStreamEnd(self.interactive)) {
                    .timeout => try self.decoder.finish(self.allocator, out),
                    .eof => {
                        try self.decoder.finishEof(self.allocator, out);
                        try out.append(self.allocator, .eof);
                    },
                }
                return;
            },
            else => return err,
        };
        if (n == 0) {
            if (self.interactive) try self.decoder.finish(self.allocator, out) else {
                try self.decoder.finishEof(self.allocator, out);
                try out.append(self.allocator, .eof);
            }
            return;
        }
        try self.decoder.feed(self.allocator, bytes[0..n], out);
    }

    fn writeAll(self: *Terminal, bytes: []const u8) !void {
        try std.Io.File.stdout().writeStreamingAll(self.io, bytes);
    }
};

const StreamEnd = enum { timeout, eof };

fn classifyStreamEnd(interactive: bool) StreamEnd {
    return if (interactive) .timeout else .eof;
}

fn queryDimensions(io: std.Io) !Dimensions {
    var size: posix.winsize = .{ .row = 0, .col = 0, .xpixel = 0, .ypixel = 0 };
    const result = (try io.operate(.{ .device_io_control = .{
        .file = std.Io.File.stdout(),
        .code = posix.T.IOCGWINSZ,
        .arg = &size,
    } })).device_io_control;
    if (result < 0 or size.col == 0 or size.row == 0) return error.TerminalSizeUnavailable;
    return .{ .columns = size.col, .lines = size.row };
}

fn dimensions(environ: *const std.process.Environ.Map) Dimensions {
    return .{ .columns = parseDimension(environ.get("COLUMNS"), 80), .lines = parseDimension(environ.get("LINES"), 24) };
}
fn parseDimension(value: ?[]const u8, fallback: usize) usize {
    const parsed = std.fmt.parseInt(usize, value orelse return fallback, 10) catch return fallback;
    return if (parsed == 0) fallback else parsed;
}

test "managed screen lifetime and frames use distinct control sequences" {
    try std.testing.expectEqualStrings("\x1b[?25l\x1b[?1049h\x1b[H\x1b[2J", enter_managed_screen);
    try std.testing.expectEqualStrings("\x1b[?25h\x1b[?1049l", leave_managed_screen);
    try std.testing.expect(std.mem.indexOf(u8, enter_managed_screen, leave_managed_screen) == null);
}

test "terminal handoff lease is exclusive and generation checked" {
    var terminal: Terminal = .{ .allocator = std.testing.allocator, .io = std.testing.io, .interactive = false, .dimensions = .{} };
    defer terminal.deinit();
    const lease = try terminal.acquireHandoff();
    try std.testing.expectError(error.TerminalHandoffBusy, terminal.acquireHandoff());
    try std.testing.expectError(error.StaleTerminalHandoff, terminal.releaseHandoff(.{ .generation = lease.generation + 1 }));
    try terminal.releaseHandoff(lease);
    const next = try terminal.acquireHandoff();
    try std.testing.expect(next.generation != lease.generation);
    try terminal.releaseHandoff(next);
}

test "idle interactive stream end is a read timeout, not EOF" {
    try std.testing.expectEqual(StreamEnd.timeout, classifyStreamEnd(true));
    try std.testing.expectEqual(StreamEnd.eof, classifyStreamEnd(false));
}

test "terminal presentation validation uses instance width and cursor limit" {
    var narrow: Terminal = .{
        .allocator = std.testing.allocator,
        .io = undefined,
        .interactive = true,
        .dimensions = .{ .columns = 17, .lines = 9 },
    };
    var valid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"abcdefghijklmnopq\"}]}],\"cursor\":{\"row\":1,\"byte\":15}}", .{});
    defer valid.deinit();
    var prepared = try narrow.preparePresentation(valid.value);
    try std.testing.expect(std.mem.startsWith(u8, prepared.bytes.items, "\x1b[?25l\x1b[H\x1b[2J"));
    prepared.deinit(std.testing.allocator);

    var too_tall = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[]},{\"spans\":[]}]}", .{});
    defer too_tall.deinit();
    const saved_lines = narrow.dimensions.lines;
    narrow.dimensions.lines = 1;
    try std.testing.expectError(error.InvalidView, narrow.preparePresentation(too_tall.value));
    narrow.dimensions.lines = saved_lines;

    var invalid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"column\":1}}", .{});
    defer invalid.deinit();
    try std.testing.expectError(error.InvalidView, narrow.preparePresentation(invalid.value));
    var past_row = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":2,\"byte\":0}}", .{});
    defer past_row.deinit();
    try std.testing.expectError(error.InvalidView, narrow.preparePresentation(past_row.value));

    narrow.interactive = false;
    narrow.dimensions.columns = 3;
    var plain_limit = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"unclipped in plain mode\"}]}],\"cursor\":{\"row\":1,\"byte\":2}}", .{});
    defer plain_limit.deinit();
    prepared = try narrow.preparePresentation(plain_limit.value);
    prepared.deinit(std.testing.allocator);
}
