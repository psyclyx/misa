//! Portable terminal lifetime, decoding, signal restoration, and inline presentation.
const std = @import("std");
const posix = std.posix;
const input = @import("input.zig");
const presenter = @import("presenter.zig");

pub const Event = input.Event;
pub const Decoder = input.Decoder;
pub const validateLines = presenter.validateLines;
const appendErase = presenter.appendErase;
const appendView = presenter.appendView;
const appendLines = presenter.appendLines;
const cursorRow = presenter.cursorRow;

pub const Dimensions = struct { columns: usize = 80, lines: usize = 24 };

const handled_signals = [_]posix.SIG{ .HUP, .INT, .QUIT, .TERM };
const SignalState = struct {
    active: bool = false,
    saved_termios: posix.termios = undefined,
    old_actions: [handled_signals.len]posix.Sigaction = undefined,
};
var signal_state: SignalState = .{};

fn restoreOnSignal(sig: posix.SIG) callconv(.c) void {
    if (!signal_state.active) return;
    posix.tcsetattr(posix.STDIN_FILENO, .NOW, signal_state.saved_termios) catch {};
    for (handled_signals, 0..) |candidate, i| {
        if (candidate == sig) {
            posix.sigaction(candidate, &signal_state.old_actions[i], null);
            posix.raise(candidate) catch {};
            return;
        }
    }
}

pub const Terminal = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    interactive: bool,
    dimensions: Dimensions,
    saved: ?posix.termios = null,
    signals_installed: bool = false,
    live_rows: usize = 0,
    live_cursor_row: usize = 0,
    decoder: Decoder = .{},

    pub fn init(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) !Terminal {
        const term = environ.get("TERM") orelse "";
        // Use the stdlib's actual tty query for classification. tcgetattr on
        // stdout is not a portable isatty substitute and fails in some real
        // terminal/build-runner combinations even though the fd is a tty.
        const input_tty = std.Io.File.stdin().isTty(io) catch false;
        const output_tty = std.Io.File.stdout().isTty(io) catch false;
        const input_attr = if (input_tty) posix.tcgetattr(posix.STDIN_FILENO) catch null else null;
        const interactive = input_attr != null and output_tty and term.len != 0 and !std.mem.eql(u8, term, "dumb");
        var self: Terminal = .{ .allocator = allocator, .io = io, .interactive = interactive, .dimensions = dimensions(environ) };
        if (interactive) {
            self.saved = input_attr.?;
            var raw = input_attr.?;
            raw.lflag.ICANON = false;
            raw.lflag.ECHO = false;
            raw.lflag.IEXTEN = false;
            raw.lflag.ISIG = false;
            raw.iflag.IXON = false;
            raw.iflag.ICRNL = false;
            raw.iflag.BRKINT = false;
            raw.iflag.INPCK = false;
            raw.iflag.ISTRIP = false;
            raw.cc[@intFromEnum(posix.V.MIN)] = 0;
            raw.cc[@intFromEnum(posix.V.TIME)] = 1;
            try posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, raw);
            errdefer posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, input_attr.?) catch {};
            signal_state.saved_termios = input_attr.?;
            var signal_mask = posix.sigemptyset();
            for (handled_signals) |sig| posix.sigaddset(&signal_mask, sig);
            var old_mask: posix.sigset_t = undefined;
            posix.sigprocmask(posix.SIG.BLOCK, &signal_mask, &old_mask);
            defer posix.sigprocmask(posix.SIG.SETMASK, &old_mask, null);
            const action: posix.Sigaction = .{ .handler = .{ .handler = restoreOnSignal }, .mask = posix.sigemptyset(), .flags = 0 };
            for (handled_signals, 0..) |sig, i| posix.sigaction(sig, &action, &signal_state.old_actions[i]);
            signal_state.active = true;
            self.signals_installed = true;
            // Bracketed paste is decoded if a parent enables it, but misa does
            // not enable it because std.posix exposes no portable async-safe write.
        }
        return self;
    }

    pub fn deinit(self: *Terminal) void {
        if (self.interactive) {
            self.eraseLive() catch {};
            if (self.saved) |saved| posix.tcsetattr(posix.STDIN_FILENO, .DRAIN, saved) catch {};
            if (self.signals_installed) {
                var signal_mask = posix.sigemptyset();
                for (handled_signals) |sig| posix.sigaddset(&signal_mask, sig);
                var old_mask: posix.sigset_t = undefined;
                posix.sigprocmask(posix.SIG.BLOCK, &signal_mask, &old_mask);
                for (handled_signals, 0..) |sig, i| posix.sigaction(sig, &signal_state.old_actions[i], null);
                // While handled signals are blocked, no handler can observe the
                // transition between restored actions and inactive state.
                signal_state.active = false;
                self.signals_installed = false;
                posix.sigprocmask(posix.SIG.SETMASK, &old_mask, null);
            }
        }
        self.decoder.deinit(self.allocator);
    }

    /// Validate with this terminal's real width and presentation mode. Session
    /// transactions call this before committing their pending Lua state.
    pub fn validatePresentation(self: *const Terminal, view: std.json.Value) !void {
        var sink: std.ArrayList(u8) = .empty;
        defer sink.deinit(self.allocator);
        _ = try self.appendPresentation(&sink, view);
    }

    pub fn present(self: *Terminal, view: std.json.Value) !void {
        var buffer: std.ArrayList(u8) = .empty;
        defer buffer.deinit(self.allocator);
        if (self.interactive) try appendErase(&buffer, self.allocator, self.live_rows, self.live_cursor_row);
        const new_rows = try self.appendPresentation(&buffer, view);
        if (!self.interactive) return;
        const new_cursor_row = try cursorRow(view, new_rows);
        try self.writeAll(buffer.items);
        self.live_rows = new_rows;
        self.live_cursor_row = new_cursor_row;
    }

    fn appendPresentation(self: *const Terminal, out: *std.ArrayList(u8), view: std.json.Value) !usize {
        return appendView(out, self.allocator, view, self.dimensions.columns, self.dimensions.lines, self.interactive, false);
    }

    pub fn commit(self: *Terminal, lines: std.json.Value) !void {
        var buffer: std.ArrayList(u8) = .empty;
        defer buffer.deinit(self.allocator);
        // Build and validate everything before touching terminal ownership.
        if (self.interactive) try appendErase(&buffer, self.allocator, self.live_rows, self.live_cursor_row);
        _ = try appendLines(&buffer, self.allocator, lines, self.dimensions.columns, self.interactive, true, false);
        try self.writeAll(buffer.items);
        self.live_rows = 0;
        self.live_cursor_row = 0;
    }

    pub fn readEvents(self: *Terminal, out: *std.ArrayList(Event)) !void {
        var bytes: [4096]u8 = undefined;
        const n = std.Io.File.stdin().readStreaming(self.io, &.{&bytes}) catch |err| switch (err) {
            error.EndOfStream => {
                // With VMIN=0/VTIME=1, an idle tty read times out with zero
                // bytes; Zig reports that as EndOfStream. It is not EOF.
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

    fn eraseLive(self: *Terminal) !void {
        if (self.live_rows == 0) return;
        var buffer: std.ArrayList(u8) = .empty;
        defer buffer.deinit(self.allocator);
        try appendErase(&buffer, self.allocator, self.live_rows, self.live_cursor_row);
        try self.writeAll(buffer.items);
        self.live_rows = 0;
        self.live_cursor_row = 0;
    }

    fn writeAll(self: *Terminal, bytes: []const u8) !void {
        try std.Io.File.stdout().writeStreamingAll(self.io, bytes);
    }
};

const StreamEnd = enum { timeout, eof };

fn classifyStreamEnd(interactive: bool) StreamEnd {
    return if (interactive) .timeout else .eof;
}

fn dimensions(environ: *const std.process.Environ.Map) Dimensions {
    return .{ .columns = parseDimension(environ.get("COLUMNS"), 80), .lines = parseDimension(environ.get("LINES"), 24) };
}
fn parseDimension(value: ?[]const u8, fallback: usize) usize {
    const parsed = std.fmt.parseInt(usize, value orelse return fallback, 10) catch return fallback;
    return if (parsed == 0) fallback else parsed;
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
    try narrow.validatePresentation(valid.value);

    var invalid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"column\":1}}", .{});
    defer invalid.deinit();
    try std.testing.expectError(error.InvalidView, narrow.validatePresentation(invalid.value));
    var past_row = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":2,\"byte\":0}}", .{});
    defer past_row.deinit();
    try std.testing.expectError(error.InvalidView, narrow.validatePresentation(past_row.value));

    narrow.interactive = false;
    narrow.dimensions.columns = 3;
    var plain_limit = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"unclipped in plain mode\"}]}],\"cursor\":{\"row\":1,\"byte\":2}}", .{});
    defer plain_limit.deinit();
    try narrow.validatePresentation(plain_limit.value);
}
