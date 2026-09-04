//! Portable terminal lifetime, decoding, signal restoration, and inline presentation.
const std = @import("std");
const posix = std.posix;

pub const Dimensions = struct { columns: usize = 80, lines: usize = 24 };

pub const Event = union(enum) {
    text: []u8,
    enter,
    backspace,
    arrow_up,
    arrow_down,
    arrow_left,
    arrow_right,
    escape,
    ctrl_c,
    eof,

    pub fn deinit(self: Event, allocator: std.mem.Allocator) void {
        if (self == .text) allocator.free(self.text);
    }
};

/// Incremental decoder. Escape prefixes, UTF-8, and bracketed paste may span reads.
pub const Decoder = struct {
    pending: std.ArrayList(u8) = .empty,
    paste: bool = false,
    paste_cr: bool = false,

    pub fn deinit(self: *Decoder, allocator: std.mem.Allocator) void {
        self.pending.deinit(allocator);
    }

    pub fn feed(self: *Decoder, allocator: std.mem.Allocator, bytes: []const u8, out: *std.ArrayList(Event)) !void {
        try self.pending.appendSlice(allocator, bytes);
        while (self.pending.items.len > 0) {
            if (self.paste) {
                if (std.mem.indexOf(u8, self.pending.items, "\x1b[201~")) |end| {
                    try self.emitPaste(allocator, self.pending.items[0..end], true, out);
                    self.consume(end + 6);
                    self.paste = false;
                    continue;
                }
                // Retain enough bytes for a fragmented end marker and UTF-8 sequence.
                if (self.pending.items.len <= 9) return;
                var n = self.pending.items.len - 9;
                while (n > 0 and n < self.pending.items.len and self.pending.items[n] & 0xc0 == 0x80) n -= 1;
                if (n == 0) return;
                try self.emitPaste(allocator, self.pending.items[0..n], false, out);
                self.consume(n);
                continue;
            }
            const p = self.pending.items;
            if (p[0] == 0x1b) {
                const sequences = [_]struct { bytes: []const u8, event: Event }{
                    .{ .bytes = "\x1b[A", .event = .arrow_up },    .{ .bytes = "\x1b[B", .event = .arrow_down },
                    .{ .bytes = "\x1b[C", .event = .arrow_right }, .{ .bytes = "\x1b[D", .event = .arrow_left },
                };
                if (std.mem.startsWith(u8, p, "\x1b[200~")) {
                    if (p.len < 6) return;
                    self.consume(6);
                    self.paste = true;
                    continue;
                }
                var prefix = false;
                for (sequences) |seq| {
                    if (std.mem.startsWith(u8, p, seq.bytes)) {
                        self.consume(seq.bytes.len);
                        try out.append(allocator, seq.event);
                        prefix = false;
                        break;
                    }
                    if (std.mem.startsWith(u8, seq.bytes, p)) prefix = true;
                } else {
                    if (prefix or std.mem.startsWith(u8, "\x1b[200~", p)) return;
                    self.consume(1);
                    try out.append(allocator, .escape);
                }
                continue;
            }
            const byte = p[0];
            if (byte == 3) {
                self.consume(1);
                try out.append(allocator, .ctrl_c);
                continue;
            }
            if (byte == 4) {
                self.consume(1);
                try out.append(allocator, .eof);
                continue;
            }
            if (byte == '\r') {
                // Hold CR until LF arrives or the terminal read timeout expires,
                // so fragmented and unfragmented CRLF both become one Enter.
                if (p.len == 1) return;
                self.consume(if (p[1] == '\n') 2 else 1);
                try out.append(allocator, .enter);
                continue;
            }
            if (byte == '\n') {
                self.consume(1);
                try out.append(allocator, .enter);
                continue;
            }
            if (byte == 0x7f or byte == 8) {
                self.consume(1);
                try out.append(allocator, .backspace);
                continue;
            }
            if (byte < 0x20 or (byte >= 0x80 and byte <= 0x9f)) {
                self.consume(1);
                continue;
            }
            const len = std.unicode.utf8ByteSequenceLength(byte) catch {
                self.consume(1);
                continue;
            };
            if (p.len < len) return;
            const cp = std.unicode.utf8Decode(p[0..len]) catch {
                self.consume(1);
                continue;
            };
            if (cp >= 0x80 and cp <= 0x9f) {
                self.consume(len);
                continue;
            }
            try out.append(allocator, .{ .text = try allocator.dupe(u8, p[0..len]) });
            self.consume(len);
        }
    }

    /// Resolve timeout-ambiguous Escape prefixes and standalone CR. An
    /// incomplete escape sequence becomes Escape followed by its safely
    /// decoded remainder rather than remaining pending forever.
    pub fn finish(self: *Decoder, allocator: std.mem.Allocator, out: *std.ArrayList(Event)) !void {
        if (self.paste) return;
        if (self.pending.items.len > 0 and self.pending.items[0] == 0x1b) {
            self.consume(1);
            try out.append(allocator, .escape);
            try self.feed(allocator, "", out);
        }
        if (self.pending.items.len > 0 and self.pending.items[0] == '\r') {
            self.consume(if (self.pending.items.len > 1 and self.pending.items[1] == '\n') 2 else 1);
            try out.append(allocator, .enter);
            try self.feed(allocator, "", out);
        }
    }

    fn finishEof(self: *Decoder, allocator: std.mem.Allocator, out: *std.ArrayList(Event)) !void {
        if (self.paste) {
            try self.emitPaste(allocator, self.pending.items, true, out);
            self.pending.clearRetainingCapacity();
            self.paste = false;
        } else try self.finish(allocator, out);
    }

    fn emitPaste(self: *Decoder, allocator: std.mem.Allocator, bytes: []const u8, final: bool, out: *std.ArrayList(Event)) !void {
        var clean: std.ArrayList(u8) = .empty;
        errdefer clean.deinit(allocator);
        var i: usize = 0;
        while (i < bytes.len) {
            const byte = bytes[i];
            if (self.paste_cr) {
                try clean.append(allocator, '\n');
                self.paste_cr = false;
                if (byte == '\n') {
                    i += 1;
                    continue;
                }
            }
            if (byte == '\r') {
                self.paste_cr = true;
                i += 1;
                continue;
            }
            if (byte == '\n') {
                try clean.append(allocator, '\n');
                i += 1;
                continue;
            }
            if (byte == '\t') {
                try clean.append(allocator, ' ');
                i += 1;
                continue;
            }
            if (byte == 0x1b or byte < 0x20 or byte == 0x7f) {
                i += 1;
                continue;
            }
            const len = std.unicode.utf8ByteSequenceLength(byte) catch {
                try clean.appendSlice(allocator, "\xef\xbf\xbd");
                i += 1;
                continue;
            };
            if (i + len > bytes.len) {
                if (!final) break;
                try clean.appendSlice(allocator, "\xef\xbf\xbd");
                break;
            }
            const cp = std.unicode.utf8Decode(bytes[i .. i + len]) catch {
                try clean.appendSlice(allocator, "\xef\xbf\xbd");
                i += 1;
                continue;
            };
            if (cp >= 0x80 and cp <= 0x9f) try clean.appendSlice(allocator, "\xef\xbf\xbd") else try clean.appendSlice(allocator, bytes[i .. i + len]);
            i += len;
        }
        if (final and self.paste_cr) {
            try clean.append(allocator, '\n');
            self.paste_cr = false;
        }
        if (clean.items.len != 0) try out.append(allocator, .{ .text = try clean.toOwnedSlice(allocator) }) else clean.deinit(allocator);
    }

    fn consume(self: *Decoder, n: usize) void {
        std.mem.copyForwards(u8, self.pending.items[0 .. self.pending.items.len - n], self.pending.items[n..]);
        self.pending.items.len -= n;
    }
};

const handled_signals = [_]posix.SIG{ .HUP, .INT, .QUIT, .TERM, .ABRT, .SEGV, .ILL, .FPE };
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
        const input_attr = posix.tcgetattr(posix.STDIN_FILENO) catch null;
        const output_tty = (posix.tcgetattr(posix.STDOUT_FILENO) catch null) != null;
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
        const n = try std.Io.File.stdin().readStreaming(self.io, &.{&bytes});
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

fn dimensions(environ: *const std.process.Environ.Map) Dimensions {
    return .{ .columns = parseDimension(environ.get("COLUMNS"), 80), .lines = parseDimension(environ.get("LINES"), 24) };
}
fn parseDimension(value: ?[]const u8, fallback: usize) usize {
    const parsed = std.fmt.parseInt(usize, value orelse return fallback, 10) catch return fallback;
    return if (parsed == 0) fallback else parsed;
}
fn appendErase(out: *std.ArrayList(u8), allocator: std.mem.Allocator, rows: usize, cursor_row: usize) !void {
    if (rows == 0) return;
    if (cursor_row > 0 and cursor_row < rows) try out.print(allocator, "\x1b[{d}B", .{rows - cursor_row});
    try out.appendSlice(allocator, "\r\x1b[2K");
    var remaining = rows - 1;
    while (remaining > 0) : (remaining -= 1) try out.appendSlice(allocator, "\x1b[1A\r\x1b[2K");
}

const style_codes = std.StaticStringMap([]const u8).initComptime(.{
    .{ "plain", "\x1b[0m" }, .{ "dim", "\x1b[2m" },        .{ "bold", "\x1b[1m" },   .{ "accent", "\x1b[36m" },
    .{ "user", "\x1b[32m" }, .{ "assistant", "\x1b[35m" }, .{ "error", "\x1b[31m" },
});

pub fn validateLines(lines: std.json.Value) !void {
    var sink: std.ArrayList(u8) = .empty;
    defer sink.deinit(std.heap.page_allocator);
    _ = try appendLines(&sink, std.heap.page_allocator, lines, 80, false, false, false);
}

fn appendView(out: *std.ArrayList(u8), allocator: std.mem.Allocator, view: std.json.Value, columns: usize, max_lines: usize, ansi: bool, final_newline: bool) !usize {
    const object = switch (view) {
        .object => |o| o,
        else => return error.InvalidView,
    };
    const lines = object.get("lines") orelse return error.InvalidView;
    const row_count = try appendLines(out, allocator, lines, columns, ansi, final_newline, true);
    if (try resolveCursor(object.get("cursor"), lines, row_count, columns, max_lines)) |cursor| {
        if (ansi and row_count != 0) {
            if (row_count > cursor.row) try out.print(allocator, "\x1b[{d}A", .{row_count - cursor.row});
            try out.appendSlice(allocator, "\r");
            if (cursor.column > 1) try out.print(allocator, "\x1b[{d}C", .{cursor.column - 1});
        }
    }
    return row_count;
}

const Cursor = struct { row: usize, column: usize };

/// A cursor uses a one-based semantic row and either a one-based terminal
/// column or a zero-based UTF-8 byte offset into that row's concatenated spans.
fn resolveCursor(value: ?std.json.Value, lines_value: std.json.Value, row_count: usize, columns: usize, max_lines: usize) !?Cursor {
    const cursor = value orelse return null;
    const position = switch (cursor) {
        .null => return null,
        .object => |object| object,
        else => return error.InvalidView,
    };
    const row = try coordinate(position.get("row"));
    if (row > row_count or row > max_lines) return error.InvalidView;
    const has_column = position.get("column") != null;
    const has_byte = position.get("byte") != null;
    if (has_column == has_byte) return error.InvalidView;
    const limit = @max(@as(usize, 1), columns -| 1);
    if (has_column) {
        const column = try coordinate(position.get("column"));
        const rendered_endpoint = @min((try rowDisplayWidth(lines_value, row)) +| 1, limit);
        if (column > rendered_endpoint) return error.InvalidView;
        return .{ .row = row, .column = column };
    }
    const byte = try byteOffset(position.get("byte"));
    const width = try widthAtByteOffset(lines_value, row, byte);
    return .{ .row = row, .column = @min(width +| 1, limit) };
}

fn widthAtByteOffset(lines_value: std.json.Value, row: usize, target: usize) !usize {
    const lines = switch (lines_value) {
        .array => |array| array.items,
        else => return error.InvalidView,
    };
    if (row == 0 or row > lines.len) return error.InvalidView;
    const line = switch (lines[row - 1]) {
        .object => |object| object,
        else => return error.InvalidView,
    };
    const spans = switch (line.get("spans") orelse return error.InvalidView) {
        .array => |array| array.items,
        else => return error.InvalidView,
    };
    var offset: usize = 0;
    var width: usize = 0;
    for (spans) |span| {
        const object = switch (span) {
            .object => |item| item,
            else => return error.InvalidView,
        };
        const text = switch (object.get("text") orelse return error.InvalidView) {
            .string => |string| string,
            else => return error.InvalidView,
        };
        var iterator = std.unicode.Utf8Iterator{ .bytes = text, .i = 0 };
        while (iterator.nextCodepointSlice()) |encoded| {
            if (target == offset) return width;
            if (target < offset + encoded.len) return error.InvalidView;
            width += displayWidth(std.unicode.utf8Decode(encoded) catch return error.InvalidView);
            offset += encoded.len;
        }
    }
    if (target != offset) return error.InvalidView;
    return width;
}

fn rowDisplayWidth(lines_value: std.json.Value, row: usize) !usize {
    const lines = switch (lines_value) {
        .array => |array| array.items,
        else => return error.InvalidView,
    };
    if (row == 0 or row > lines.len) return error.InvalidView;
    const line = switch (lines[row - 1]) {
        .object => |object| object,
        else => return error.InvalidView,
    };
    const spans = switch (line.get("spans") orelse return error.InvalidView) {
        .array => |array| array.items,
        else => return error.InvalidView,
    };
    var width: usize = 0;
    for (spans) |span| {
        const object = switch (span) {
            .object => |item| item,
            else => return error.InvalidView,
        };
        const text = switch (object.get("text") orelse return error.InvalidView) {
            .string => |string| string,
            else => return error.InvalidView,
        };
        var iterator = std.unicode.Utf8Iterator{ .bytes = text, .i = 0 };
        while (iterator.nextCodepoint()) |cp| width += displayWidth(cp);
    }
    return width;
}

fn byteOffset(value: ?std.json.Value) !usize {
    const number = switch (value orelse return error.InvalidView) {
        .integer => |integer| integer,
        else => return error.InvalidView,
    };
    if (number < 0) return error.InvalidView;
    return @intCast(number);
}

fn appendLines(out: *std.ArrayList(u8), allocator: std.mem.Allocator, value: std.json.Value, columns: usize, ansi: bool, final_newline: bool, clip: bool) !usize {
    const lines = switch (value) {
        .array => |v| v.items,
        else => return error.InvalidView,
    };
    const limit = if (columns > 1) columns - 1 else 0;
    for (lines, 0..) |line, line_index| {
        const object = switch (line) {
            .object => |o| o,
            else => return error.InvalidView,
        };
        const spans = switch (object.get("spans") orelse return error.InvalidView) {
            .array => |v| v.items,
            else => return error.InvalidView,
        };
        var width: usize = 0;
        var clipped = false;
        for (spans) |span| {
            const span_object = switch (span) {
                .object => |o| o,
                else => return error.InvalidView,
            };
            const text = switch (span_object.get("text") orelse return error.InvalidView) {
                .string => |s| s,
                else => return error.InvalidView,
            };
            const style = if (span_object.get("style")) |style_value| switch (style_value) {
                .string => |s| s,
                else => return error.InvalidView,
            } else "plain";
            const code = style_codes.get(style) orelse return error.InvalidView;
            if (!std.unicode.utf8ValidateSlice(text)) return error.InvalidView;
            if (ansi and !clipped) try out.appendSlice(allocator, code);
            var iterator = std.unicode.Utf8Iterator{ .bytes = text, .i = 0 };
            while (iterator.nextCodepointSlice()) |encoded| {
                const cp = std.unicode.utf8Decode(encoded) catch return error.InvalidView;
                if (cp == 0x1b or cp == '\n' or cp == '\r' or cp < 0x20 or (cp >= 0x7f and cp <= 0x9f)) return error.InvalidView;
                const char_width = displayWidth(cp);
                if (clip and (limit == 0 or width + char_width > limit)) {
                    clipped = true;
                    continue;
                }
                if (!clipped) try out.appendSlice(allocator, encoded);
                width += char_width;
            }
        }
        if (ansi) try out.appendSlice(allocator, "\x1b[0m");
        if (line_index + 1 < lines.len or final_newline) try out.appendSlice(allocator, "\n");
    }
    return lines.len;
}

fn cursorRow(view: std.json.Value, rows: usize) !usize {
    const object = switch (view) {
        .object => |o| o,
        else => return error.InvalidView,
    };
    if (object.get("cursor")) |cursor| switch (cursor) {
        .null => {},
        .object => |position| return coordinate(position.get("row")),
        else => return error.InvalidView,
    };
    return rows;
}

fn coordinate(value: ?std.json.Value) !usize {
    const number = switch (value orelse return error.InvalidView) {
        .integer => |n| n,
        else => return error.InvalidView,
    };
    if (number < 1) return error.InvalidView;
    return @intCast(number);
}

const WidthInterval = struct { first: u21, last: u21 };

fn inIntervals(cp: u21, comptime intervals: []const WidthInterval) bool {
    var low: usize = 0;
    var high: usize = intervals.len;
    while (low < high) {
        const middle = low + (high - low) / 2;
        const interval = intervals[middle];
        if (cp < interval.first) high = middle else if (cp > interval.last) low = middle + 1 else return true;
    }
    return false;
}

// Local wcwidth-style tables. Unlisted printable Unicode is one cell; this is
// more accurate than treating every non-ASCII codepoint as double-width.
const zero_width = [_]WidthInterval{
    .{ .first = 0x0300, .last = 0x036f },   .{ .first = 0x0483, .last = 0x0489 },
    .{ .first = 0x0591, .last = 0x05bd },   .{ .first = 0x05bf, .last = 0x05bf },
    .{ .first = 0x05c1, .last = 0x05c2 },   .{ .first = 0x05c4, .last = 0x05c5 },
    .{ .first = 0x0610, .last = 0x061a },   .{ .first = 0x064b, .last = 0x065f },
    .{ .first = 0x0670, .last = 0x0670 },   .{ .first = 0x06d6, .last = 0x06ed },
    .{ .first = 0x0711, .last = 0x0711 },   .{ .first = 0x0730, .last = 0x074a },
    .{ .first = 0x07a6, .last = 0x07b0 },   .{ .first = 0x07eb, .last = 0x07f3 },
    .{ .first = 0x0816, .last = 0x082d },   .{ .first = 0x0859, .last = 0x085b },
    .{ .first = 0x08d3, .last = 0x0902 },   .{ .first = 0x093a, .last = 0x093c },
    .{ .first = 0x0941, .last = 0x0948 },   .{ .first = 0x094d, .last = 0x094d },
    .{ .first = 0x0951, .last = 0x0957 },   .{ .first = 0x0962, .last = 0x0963 },
    .{ .first = 0x1ab0, .last = 0x1aff },   .{ .first = 0x1dc0, .last = 0x1dff },
    .{ .first = 0x200b, .last = 0x200f },   .{ .first = 0x202a, .last = 0x202e },
    .{ .first = 0x2060, .last = 0x206f },   .{ .first = 0x20d0, .last = 0x20ff },
    .{ .first = 0xfe00, .last = 0xfe0f },   .{ .first = 0xfe20, .last = 0xfe2f },
    .{ .first = 0xfeff, .last = 0xfeff },   .{ .first = 0x1f3fb, .last = 0x1f3ff },
    .{ .first = 0xe0020, .last = 0xe007f }, .{ .first = 0xe0100, .last = 0xe01ef },
};

const wide = [_]WidthInterval{
    .{ .first = 0x1100, .last = 0x115f },   .{ .first = 0x231a, .last = 0x231b },
    .{ .first = 0x2329, .last = 0x232a },   .{ .first = 0x23e9, .last = 0x23ec },
    .{ .first = 0x23f0, .last = 0x23f0 },   .{ .first = 0x23f3, .last = 0x23f3 },
    .{ .first = 0x25fd, .last = 0x25fe },   .{ .first = 0x2614, .last = 0x2615 },
    .{ .first = 0x2648, .last = 0x2653 },   .{ .first = 0x267f, .last = 0x267f },
    .{ .first = 0x2693, .last = 0x2693 },   .{ .first = 0x26a1, .last = 0x26a1 },
    .{ .first = 0x26aa, .last = 0x26ab },   .{ .first = 0x26bd, .last = 0x26be },
    .{ .first = 0x26c4, .last = 0x26c5 },   .{ .first = 0x26ce, .last = 0x26ce },
    .{ .first = 0x26d4, .last = 0x26d4 },   .{ .first = 0x26ea, .last = 0x26ea },
    .{ .first = 0x26f2, .last = 0x26f3 },   .{ .first = 0x26f5, .last = 0x26f5 },
    .{ .first = 0x26fa, .last = 0x26fa },   .{ .first = 0x26fd, .last = 0x26fd },
    .{ .first = 0x2705, .last = 0x2705 },   .{ .first = 0x270a, .last = 0x270b },
    .{ .first = 0x2728, .last = 0x2728 },   .{ .first = 0x274c, .last = 0x274c },
    .{ .first = 0x274e, .last = 0x274e },   .{ .first = 0x2753, .last = 0x2755 },
    .{ .first = 0x2757, .last = 0x2757 },   .{ .first = 0x2795, .last = 0x2797 },
    .{ .first = 0x27b0, .last = 0x27b0 },   .{ .first = 0x27bf, .last = 0x27bf },
    .{ .first = 0x2b1b, .last = 0x2b1c },   .{ .first = 0x2b50, .last = 0x2b50 },
    .{ .first = 0x2b55, .last = 0x2b55 },   .{ .first = 0x2e80, .last = 0x303e },
    .{ .first = 0x3040, .last = 0xa4cf },   .{ .first = 0xac00, .last = 0xd7a3 },
    .{ .first = 0xf900, .last = 0xfaff },   .{ .first = 0xfe10, .last = 0xfe19 },
    .{ .first = 0xfe30, .last = 0xfe6f },   .{ .first = 0xff00, .last = 0xff60 },
    .{ .first = 0xffe0, .last = 0xffe6 },   .{ .first = 0x1f004, .last = 0x1f004 },
    .{ .first = 0x1f0cf, .last = 0x1f0cf }, .{ .first = 0x1f18e, .last = 0x1f18e },
    .{ .first = 0x1f191, .last = 0x1f19a }, .{ .first = 0x1f200, .last = 0x1f251 },
    .{ .first = 0x1f300, .last = 0x1f64f }, .{ .first = 0x1f680, .last = 0x1f6ff },
    .{ .first = 0x1f900, .last = 0x1f9ff }, .{ .first = 0x1fa70, .last = 0x1faff },
    .{ .first = 0x20000, .last = 0x3fffd },
};

pub fn displayWidth(cp: u21) usize {
    if (inIntervals(cp, &zero_width)) return 0;
    return if (inIntervals(cp, &wide)) 2 else 1;
}

pub fn renderForTest(allocator: std.mem.Allocator, previous_rows: usize, view: std.json.Value, commit: bool) ![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    try appendErase(&out, allocator, previous_rows, previous_rows);
    _ = try appendView(&out, allocator, view, 80, 24, true, commit);
    return out.toOwnedSlice(allocator);
}

pub fn ownershipAfterWrite(previous: usize, proposed: usize, write_succeeded: bool) usize {
    return if (write_succeeded) proposed else previous;
}

test "presenter erases exact rows, honors cursor, and forbids screen clears" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"hello\",\"style\":\"accent\"}]},{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"column\":3}}", .{});
    defer parsed.deinit();
    const frame = try renderForTest(std.testing.allocator, 2, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.startsWith(u8, frame, "\r\x1b[2K\x1b[1A\r\x1b[2K"));
    try std.testing.expect(std.mem.endsWith(u8, frame, "\x1b[1A\r\x1b[2C"));
    for ([_][]const u8{ "\x1b[J", "\x1b[2J", "\x1b[3J", "?1049" }) |forbidden| try std.testing.expect(std.mem.indexOf(u8, frame, forbidden) == null);
    try std.testing.expectEqual(@as(usize, 4), ownershipAfterWrite(4, 2, false));
    try std.testing.expectEqual(@as(usize, 2), ownershipAfterWrite(4, 2, true));
}

test "presenter converts semantic UTF-8 byte cursors across spans" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"> \"},{\"text\":\"aéz\"}]}],\"cursor\":{\"row\":1,\"byte\":5}}", .{});
    defer parsed.deinit();
    const frame = try renderForTest(std.testing.allocator, 0, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.endsWith(u8, frame, "\r\x1b[4C"));
    var narrow: std.ArrayList(u8) = .empty;
    defer narrow.deinit(std.testing.allocator);
    _ = try appendView(&narrow, std.testing.allocator, parsed.value, 4, 24, true, false);
    try std.testing.expect(std.mem.endsWith(u8, narrow.items, "\r\x1b[2C"));

    var split = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"é\"}]}],\"cursor\":{\"row\":1,\"byte\":1}}", .{});
    defer split.deinit();
    try std.testing.expectError(error.InvalidView, renderForTest(std.testing.allocator, 0, split.value, false));
}

test "display width handles narrow scripts, combining sequences, and wide glyphs" {
    try std.testing.expectEqual(@as(usize, 1), displayWidth('é'));
    try std.testing.expectEqual(@as(usize, 1), displayWidth('Ω'));
    try std.testing.expectEqual(@as(usize, 1), displayWidth('Ж'));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0x0301));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0xfe0f));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0x200d));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0x1f3fb));
    try std.testing.expectEqual(@as(usize, 2), displayWidth('界'));
    try std.testing.expectEqual(@as(usize, 2), displayWidth(0x1f600));

    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"é界\"}]}],\"cursor\":{\"row\":1,\"byte\":6}}", .{});
    defer parsed.deinit();
    const frame = try renderForTest(std.testing.allocator, 0, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.endsWith(u8, frame, "\r\x1b[3C"));
}

test "decoder handles fragmented safe multiline paste, escape timeout, ctrl-d, and utf8" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer {
        for (events.items) |event| event.deinit(std.testing.allocator);
        events.deinit(std.testing.allocator);
    }
    try decoder.feed(std.testing.allocator, "\x1b[200~a\r", &events);
    try decoder.feed(std.testing.allocator, "\nb\t\x1bX\xc2\x85\xe2", &events);
    try decoder.feed(std.testing.allocator, "\x98\x83\x1b[201~\x1b", &events);
    try decoder.finish(std.testing.allocator, &events);
    try decoder.feed(std.testing.allocator, "\x04\x01", &events);
    var joined: std.ArrayList(u8) = .empty;
    defer joined.deinit(std.testing.allocator);
    var saw_escape = false;
    var saw_eof = false;
    for (events.items) |event| switch (event) {
        .text => |text| try joined.appendSlice(std.testing.allocator, text),
        .escape => saw_escape = true,
        .eof => saw_eof = true,
        else => {},
    };
    try std.testing.expectEqualStrings("a\nb X�☃", joined.items);
    try std.testing.expect(saw_escape and saw_eof);
}

test "decoder timeout resolves partial escape sequences and normalizes CRLF" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer {
        for (events.items) |event| event.deinit(std.testing.allocator);
        events.deinit(std.testing.allocator);
    }

    try decoder.feed(std.testing.allocator, "\x1b[2", &events);
    try std.testing.expectEqual(@as(usize, 0), events.items.len);
    try decoder.finish(std.testing.allocator, &events);
    try decoder.feed(std.testing.allocator, "\r\n", &events);
    try decoder.feed(std.testing.allocator, "\r", &events);
    try decoder.feed(std.testing.allocator, "\n\r", &events);
    try decoder.finish(std.testing.allocator, &events);

    try std.testing.expectEqual(@as(usize, 6), events.items.len);
    try std.testing.expect(events.items[0] == .escape);
    try std.testing.expectEqualStrings("[", events.items[1].text);
    try std.testing.expectEqualStrings("2", events.items[2].text);
    try std.testing.expect(events.items[3] == .enter);
    try std.testing.expect(events.items[4] == .enter);
    try std.testing.expect(events.items[5] == .enter);
}

test "terminal presentation validation uses instance width and cursor limit" {
    var narrow: Terminal = .{
        .allocator = std.testing.allocator,
        .io = undefined,
        .interactive = true,
        .dimensions = .{ .columns = 17, .lines = 9 },
    };
    var valid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"abcdefghijklmnopq\"}]}],\"cursor\":{\"row\":1,\"column\":16}}", .{});
    defer valid.deinit();
    try narrow.validatePresentation(valid.value);

    var invalid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"column\":17}}", .{});
    defer invalid.deinit();
    try std.testing.expectError(error.InvalidView, narrow.validatePresentation(invalid.value));
    var past_row = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"column\":3}}", .{});
    defer past_row.deinit();
    try std.testing.expectError(error.InvalidView, narrow.validatePresentation(past_row.value));

    narrow.interactive = false;
    narrow.dimensions.columns = 3;
    var plain_limit = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"unclipped in plain mode\"}]}],\"cursor\":{\"row\":1,\"column\":3}}", .{});
    defer plain_limit.deinit();
    try std.testing.expectError(error.InvalidView, narrow.validatePresentation(plain_limit.value));
}
