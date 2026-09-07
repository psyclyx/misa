//! Incremental terminal input decoding.
const std = @import("std");

pub const Event = union(enum) {
    text: []u8,
    alt: []u8,
    enter,
    shift_enter,
    alt_enter,
    backspace,
    tab,
    f1,
    arrow_up,
    arrow_down,
    arrow_left,
    arrow_right,
    page_up,
    page_down,
    wheel_up,
    wheel_down,
    mouse: struct { row: usize, column: usize },
    mouse_move: struct { row: usize, column: usize },
    escape,
    ctrl_c,
    ctrl_d,
    ctrl_r,
    ctrl_p,
    ctrl_n,
    ctrl_v,
    eof,

    pub fn deinit(self: Event, allocator: std.mem.Allocator) void {
        switch (self) {
            .text, .alt => |text| allocator.free(text),
            else => {},
        }
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

    /// True when a short ambiguity deadline is required. Bracketed paste is
    /// deliberately excluded: it waits for bytes or EOF, not an ESC timeout.
    pub fn needsTimeout(self: *const Decoder) bool {
        return !self.paste and self.pending.items.len != 0 and
            (self.pending.items[0] == 0x1b or self.pending.items[0] == '\r');
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
                if (std.mem.startsWith(u8, p, "\x1b[<")) {
                    const end = std.mem.indexOfAnyPos(u8, p, 3, "Mm") orelse {
                        if (p.len > 64) self.consume(p.len);
                        return;
                    };
                    var fields = std.mem.splitScalar(u8, p[3..end], ';');
                    const button = std.fmt.parseInt(usize, fields.next() orelse "", 10) catch 999;
                    const column = std.fmt.parseInt(usize, fields.next() orelse "", 10) catch 0;
                    const row = std.fmt.parseInt(usize, fields.next() orelse "", 10) catch 0;
                    if (fields.next() == null and row > 0 and column > 0 and button < 128 and p[end] == 'M') {
                        if (button & 64 != 0 and button & 3 <= 1 and button < 128) {
                            try out.append(allocator, if (button & 1 == 0) .wheel_up else .wheel_down);
                        } else if (button & 64 == 0 and button & 32 != 0) {
                            try out.append(allocator, .{ .mouse_move = .{ .row = row, .column = column } });
                        } else if (p[end] == 'M' and button & 35 == 0) {
                            try out.append(allocator, .{ .mouse = .{ .row = row, .column = column } });
                        }
                    }
                    self.consume(end + 1);
                    continue;
                }
                if (std.mem.startsWith(u8, p, "\x1b[") and p.len > 2 and std.ascii.isDigit(p[2])) {
                    var end: usize = 2;
                    while (end < p.len and (std.ascii.isDigit(p[end]) or p[end] == ';' or p[end] == ':')) : (end += 1) {}
                    if (end == p.len) return;
                    if (p[end] == 'u') {
                        try appendExtendedKey(allocator, out, p[2..end]);
                        self.consume(end + 1);
                        continue;
                    }
                    const arrow: ?Event = switch (p[end]) {
                        'A' => .arrow_up,
                        'B' => .arrow_down,
                        'C' => .arrow_right,
                        'D' => .arrow_left,
                        else => null,
                    };
                    if (arrow) |event| {
                        try out.append(allocator, event);
                        self.consume(end + 1);
                        continue;
                    }
                }
                const sequences = [_]struct { bytes: []const u8, event: Event }{
                    .{ .bytes = "\x1b[13;2u", .event = .shift_enter }, .{ .bytes = "\x1b[27;2;13~", .event = .shift_enter },
                    .{ .bytes = "\x1b[13;3u", .event = .alt_enter },   .{ .bytes = "\x1b\r", .event = .alt_enter },
                    .{ .bytes = "\x1bOP", .event = .f1 },              .{ .bytes = "\x1b[11~", .event = .f1 },
                    .{ .bytes = "\x1b[A", .event = .arrow_up },        .{ .bytes = "\x1b[B", .event = .arrow_down },
                    .{ .bytes = "\x1b[C", .event = .arrow_right },     .{ .bytes = "\x1b[D", .event = .arrow_left },
                    .{ .bytes = "\x1b[5~", .event = .page_up },        .{ .bytes = "\x1b[6~", .event = .page_down },
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
                    switch (altPrintable(p[1..])) {
                        .incomplete => return,
                        .printable => |len| {
                            try appendOwned(allocator, out, .alt, p[1 .. len + 1]);
                            self.consume(len + 1);
                        },
                        .control => {
                            self.consume(1);
                            try out.append(allocator, .escape);
                        },
                    }
                }
                continue;
            }
            const byte = p[0];
            if (byte == 16 or byte == 14 or byte == 22) {
                self.consume(1);
                try out.append(allocator, if (byte == 16) .ctrl_p else if (byte == 14) .ctrl_n else .ctrl_v);
                continue;
            }
            if (byte == 3) {
                self.consume(1);
                try out.append(allocator, .ctrl_c);
                continue;
            }
            if (byte == 18) {
                self.consume(1);
                try out.append(allocator, .ctrl_r);
                continue;
            }
            if (byte == 4) {
                self.consume(1);
                try out.append(allocator, .ctrl_d);
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
            if (byte == '\t') {
                self.consume(1);
                try out.append(allocator, .tab);
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
            try appendOwned(allocator, out, .text, p[0..len]);
            self.consume(len);
        }
    }

    /// Resolve timeout-ambiguous Escape prefixes and standalone CR. An
    /// incomplete escape sequence becomes Escape followed by its safely
    /// decoded remainder rather than remaining pending forever.
    pub fn finish(self: *Decoder, allocator: std.mem.Allocator, out: *std.ArrayList(Event)) !void {
        if (self.paste) return;
        // Partial mouse reports are transport data, never editor text.
        if (std.mem.startsWith(u8, self.pending.items, "\x1b[<")) {
            self.pending.clearRetainingCapacity();
            return;
        }
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

    pub fn finishEof(self: *Decoder, allocator: std.mem.Allocator, out: *std.ArrayList(Event)) !void {
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
        if (clean.items.len != 0) {
            const owned = try clean.toOwnedSlice(allocator);
            errdefer allocator.free(owned);
            try out.append(allocator, .{ .text = owned });
        } else clean.deinit(allocator);
    }

    fn consume(self: *Decoder, n: usize) void {
        std.mem.copyForwards(u8, self.pending.items[0 .. self.pending.items.len - n], self.pending.items[n..]);
        self.pending.items.len -= n;
    }
};

fn appendOwned(allocator: std.mem.Allocator, out: *std.ArrayList(Event), comptime tag: std.meta.Tag(Event), bytes: []const u8) !void {
    const owned = try allocator.dupe(u8, bytes);
    errdefer allocator.free(owned);
    try out.append(allocator, @unionInit(Event, @tagName(tag), owned));
}

const AltPrintable = union(enum) { incomplete, control, printable: usize };

fn altPrintable(bytes: []const u8) AltPrintable {
    if (bytes.len == 0) return .incomplete;
    const len = std.unicode.utf8ByteSequenceLength(bytes[0]) catch return .control;
    if (bytes.len < len) return .incomplete;
    const cp = std.unicode.utf8Decode(bytes[0..len]) catch return .control;
    if (cp < 0x20 or cp == 0x7f or (cp >= 0x80 and cp <= 0x9f)) return .control;
    return .{ .printable = len };
}

test "SGR motion distinguishes hover drag click release and wheel" {
    const allocator = std.testing.allocator;
    var decoder: Decoder = .{};
    defer decoder.deinit(allocator);
    var events: std.ArrayList(Event) = .empty;
    defer {
        for (events.items) |event| event.deinit(allocator);
        events.deinit(allocator);
    }
    try decoder.feed(allocator, "\x1b[<35;7;", &events);
    try std.testing.expectEqual(@as(usize, 0), events.items.len);
    try decoder.feed(allocator, "3M\x1b[<32;8;3M\x1b[<0;8;3M\x1b[<0;8;3m\x1b[<64;8;3M\x1b[<999;8;3M", &events);
    try std.testing.expectEqual(@as(usize, 4), events.items.len);
    try std.testing.expect(events.items[0] == .mouse_move);
    try std.testing.expectEqual(@as(usize, 7), events.items[0].mouse_move.column);
    try std.testing.expectEqual(@as(usize, 3), events.items[0].mouse_move.row);
    try std.testing.expect(events.items[1] == .mouse_move);
    try std.testing.expect(events.items[2] == .mouse);
    try std.testing.expect(events.items[3] == .wheel_up);
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
    var saw_ctrl_d = false;
    for (events.items) |event| switch (event) {
        .text => |text| try joined.appendSlice(std.testing.allocator, text),
        .escape => saw_escape = true,
        .ctrl_d => saw_ctrl_d = true,
        else => {},
    };
    try std.testing.expectEqualStrings("a\nb X�☃", joined.items);
    try std.testing.expect(saw_escape and saw_ctrl_d);
}

test "decoder normalizes Alt printable chords without changing Escape or arrows" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer {
        for (events.items) |event| event.deinit(std.testing.allocator);
        events.deinit(std.testing.allocator);
    }

    try decoder.feed(std.testing.allocator, "\x1b1\x1b", &events);
    try decoder.feed(std.testing.allocator, "é\x1b[A\x1b", &events);
    try decoder.finish(std.testing.allocator, &events);

    try std.testing.expectEqual(@as(usize, 4), events.items.len);
    try std.testing.expectEqualStrings("1", events.items[0].alt);
    try std.testing.expectEqualStrings("é", events.items[1].alt);
    try std.testing.expect(events.items[2] == .arrow_up);
    try std.testing.expect(events.items[3] == .escape);
}

test "decoder exposes page navigation keys" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer events.deinit(std.testing.allocator);
    try decoder.feed(std.testing.allocator, "\x1b[5", &events);
    try std.testing.expectEqual(@as(usize, 0), events.items.len);
    try decoder.feed(std.testing.allocator, "~\x1b[6~", &events);
    try std.testing.expectEqual(@as(usize, 2), events.items.len);
    try std.testing.expect(events.items[0] == .page_up);
    try std.testing.expect(events.items[1] == .page_down);
}

test "decoder exposes tab as a semantic event" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer events.deinit(std.testing.allocator);
    try decoder.feed(std.testing.allocator, "\t", &events);
    try std.testing.expectEqual(@as(usize, 1), events.items.len);
    try std.testing.expect(events.items[0] == .tab);
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

test "decoder accepts fragmented F1 encodings" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer {
        for (events.items) |event| event.deinit(std.testing.allocator);
        events.deinit(std.testing.allocator);
    }
    try decoder.feed(std.testing.allocator, "\x1bO", &events);
    try std.testing.expectEqual(@as(usize, 0), events.items.len);
    try decoder.feed(std.testing.allocator, "P\x1b[11", &events);
    try std.testing.expectEqual(@as(usize, 1), events.items.len);
    try decoder.feed(std.testing.allocator, "~", &events);
    try std.testing.expectEqual(@as(usize, 2), events.items.len);
    for (events.items) |event| try std.testing.expect(event == .f1);
    try decoder.feed(std.testing.allocator, "\x12", &events);
    try std.testing.expect(events.items[2] == .ctrl_r);
}

fn appendExtendedKey(allocator: std.mem.Allocator, out: *std.ArrayList(Event), encoded: []const u8) !void {
    var fields = std.mem.splitScalar(u8, encoded, ';');
    var codes = std.mem.splitScalar(u8, fields.next() orelse return, ':');
    const code = std.fmt.parseInt(u21, codes.next() orelse return, 10) catch return;
    var modifiers = std.mem.splitScalar(u8, fields.next() orelse "1", ':');
    const modifier = std.fmt.parseInt(u8, modifiers.next() orelse "1", 10) catch return;
    if (modifiers.next()) |kind| if (std.mem.eql(u8, kind, "3")) return; // release
    if (modifier == 0) return;
    const flags = modifier - 1;
    if (code == 13) {
        try out.append(allocator, if (flags & 2 != 0) .alt_enter else if (flags & 1 != 0) .shift_enter else .enter);
        return;
    }
    if (flags & 4 != 0) {
        const event: ?Event = switch (code) {
            'c', 'C' => .ctrl_c,
            'd', 'D' => .ctrl_d,
            'r', 'R' => .ctrl_r,
            'p', 'P' => .ctrl_p,
            'n', 'N' => .ctrl_n,
            'v', 'V' => .ctrl_v,
            else => null,
        };
        if (event) |value| try out.append(allocator, value);
        return;
    }
    const special: ?Event = switch (code) {
        27 => .escape,
        9 => .tab,
        127 => .backspace,
        else => null,
    };
    if (special) |event| {
        try out.append(allocator, event);
        return;
    }
    if (code < 32 or (code >= 127 and code <= 159) or (code >= 57344 and code <= 63743)) return;
    var bytes: [4]u8 = undefined;
    const len = std.unicode.utf8Encode(code, &bytes) catch return;
    if (flags & 2 != 0) try appendOwned(allocator, out, .alt, bytes[0..len]) else try appendOwned(allocator, out, .text, bytes[0..len]);
}

test "fragmented SGR mouse and modified keyboard reports never leak text" {
    var decoder: Decoder = .{};
    defer decoder.deinit(std.testing.allocator);
    var events: std.ArrayList(Event) = .empty;
    defer {
        for (events.items) |event| event.deinit(std.testing.allocator);
        events.deinit(std.testing.allocator);
    }
    try decoder.feed(std.testing.allocator, "\x1b[<0;12;", &events);
    try std.testing.expectEqual(@as(usize, 0), events.items.len);
    try decoder.feed(std.testing.allocator, "3M\x1b[<0;12;3m\x1b[<64;12;3M\x1b[<65;12;3M\x1b[13;2", &events);
    try std.testing.expectEqual(@as(usize, 3), events.items.len);
    try std.testing.expectEqual(@as(usize, 12), events.items[0].mouse.column);
    try std.testing.expectEqual(@as(usize, 3), events.items[0].mouse.row);
    try std.testing.expect(events.items[1] == .wheel_up and events.items[2] == .wheel_down);
    try decoder.feed(std.testing.allocator, "u\x1b[13;3u\x1b[112;5u\x1b[27u", &events);
    try std.testing.expect(events.items[3] == .shift_enter and events.items[4] == .alt_enter and events.items[5] == .ctrl_p and events.items[6] == .escape);
    try decoder.feed(std.testing.allocator, "\x1b[<0;", &events);
    try decoder.finish(std.testing.allocator, &events);
    try std.testing.expectEqual(@as(usize, 7), events.items.len);
}
