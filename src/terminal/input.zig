//! Incremental terminal input decoding.
const std = @import("std");

pub const Event = union(enum) {
    text: []u8,
    enter,
    backspace,
    tab,
    arrow_up,
    arrow_down,
    arrow_left,
    arrow_right,
    escape,
    ctrl_c,
    ctrl_d,
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
        if (clean.items.len != 0) try out.append(allocator, .{ .text = try clean.toOwnedSlice(allocator) }) else clean.deinit(allocator);
    }

    fn consume(self: *Decoder, n: usize) void {
        std.mem.copyForwards(u8, self.pending.items[0 .. self.pending.items.len - n], self.pending.items[n..]);
        self.pending.items.len -= n;
    }
};

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
