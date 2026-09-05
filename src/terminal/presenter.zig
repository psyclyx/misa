//! Absolute semantic frame presentation for a managed terminal screen.
const std = @import("std");
const cell_width = @import("width.zig");

pub fn appendScreenPrelude(out: *std.ArrayList(u8), allocator: std.mem.Allocator) !void {
    // Every frame starts with a hidden cursor. A semantic cursor explicitly
    // shows it again after positioning; cursor-less/busy frames remain hidden.
    try out.appendSlice(allocator, "\x1b[?25l\x1b[H\x1b[2J");
}

pub fn usableColumns(columns: usize) usize {
    return columns -| 1;
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

pub fn appendView(out: *std.ArrayList(u8), allocator: std.mem.Allocator, view: std.json.Value, columns: usize, max_lines: usize, ansi: bool, final_newline: bool) !usize {
    const object = switch (view) {
        .object => |o| o,
        else => return error.InvalidView,
    };
    const lines = object.get("lines") orelse return error.InvalidView;
    const row_count = try appendLines(out, allocator, lines, columns, ansi, final_newline, true);
    if (row_count > max_lines) return error.InvalidView;
    if (try resolveCursor(object.get("cursor"), lines, row_count, columns, max_lines)) |cursor| {
        if (ansi and row_count != 0) try out.print(allocator, "\x1b[{d};{d}H\x1b[?25h", .{ cursor.row, cursor.column });
    }
    return row_count;
}

const Cursor = struct { row: usize, column: usize };

/// A cursor uses a one-based semantic row and a zero-based UTF-8 byte offset
/// into that row's concatenated spans. Zig alone converts bytes to cells.
fn resolveCursor(value: ?std.json.Value, lines_value: std.json.Value, row_count: usize, columns: usize, max_lines: usize) !?Cursor {
    const cursor = value orelse return null;
    const position = switch (cursor) {
        .null => return null,
        .object => |object| object,
        else => return error.InvalidView,
    };
    const row = try coordinate(position.get("row"));
    if (row > row_count or row > max_lines or position.get("column") != null) return error.InvalidView;
    const width = try widthAtByteOffset(lines_value, row, try byteOffset(position.get("byte")));
    return .{ .row = row, .column = @min(width +| 1, @max(@as(usize, 1), usableColumns(columns))) };
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
    var text: std.ArrayList(u8) = .empty;
    defer text.deinit(std.heap.page_allocator);
    for (spans) |span| {
        const object = switch (span) {
            .object => |item| item,
            else => return error.InvalidView,
        };
        const value = switch (object.get("text") orelse return error.InvalidView) {
            .string => |string| string,
            else => return error.InvalidView,
        };
        try text.appendSlice(std.heap.page_allocator, value);
    }
    if (target > text.items.len) return error.InvalidView;
    var offset: usize = 0;
    var width: usize = 0;
    while (offset < text.items.len) {
        if (target == offset) return width;
        const cluster = cell_width.nextCluster(text.items, offset) catch return error.InvalidView;
        if (target < cluster.end) return error.InvalidView;
        width += cluster.width;
        offset = cluster.end;
    }
    if (target != offset) return error.InvalidView;
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

pub fn appendLines(out: *std.ArrayList(u8), allocator: std.mem.Allocator, value: std.json.Value, columns: usize, ansi: bool, final_newline: bool, clip: bool) !usize {
    const lines = switch (value) {
        .array => |v| v.items,
        else => return error.InvalidView,
    };
    const limit = usableColumns(columns);
    for (lines, 0..) |line, line_index| {
        const object = switch (line) {
            .object => |o| o,
            else => return error.InvalidView,
        };
        const spans = switch (object.get("spans") orelse return error.InvalidView) {
            .array => |v| v.items,
            else => return error.InvalidView,
        };
        // Grapheme boundaries belong to the semantic line, not to styling
        // spans. Concatenate for segmentation, then project the visible byte
        // range back through spans so a combining/ZWJ cluster remains atomic.
        var line_text: std.ArrayList(u8) = .empty;
        defer line_text.deinit(allocator);
        for (spans) |span| {
            const span_object = switch (span) {
                .object => |o| o,
                else => return error.InvalidView,
            };
            const text = switch (span_object.get("text") orelse return error.InvalidView) {
                .string => |s| s,
                else => return error.InvalidView,
            };
            if (!std.unicode.utf8ValidateSlice(text)) return error.InvalidView;
            var check = std.unicode.Utf8Iterator{ .bytes = text, .i = 0 };
            while (check.nextCodepointSlice()) |encoded| {
                const cp = std.unicode.utf8Decode(encoded) catch return error.InvalidView;
                if (cp == 0x1b or cp == '\n' or cp == '\r' or cp < 0x20 or (cp >= 0x7f and cp <= 0x9f)) return error.InvalidView;
            }
            try line_text.appendSlice(allocator, text);
        }
        var visible_end = line_text.items.len;
        var width: usize = 0;
        var at: usize = 0;
        while (at < line_text.items.len) {
            const cluster = cell_width.nextCluster(line_text.items, at) catch return error.InvalidView;
            if (clip and (limit == 0 or width + cluster.width > limit)) {
                visible_end = at;
                break;
            }
            width += cluster.width;
            at = cluster.end;
        }
        var span_start: usize = 0;
        for (spans) |span| {
            const span_object = span.object;
            const text = span_object.get("text").?.string;
            const style = if (span_object.get("style")) |style_value| switch (style_value) {
                .string => |s| s,
                else => return error.InvalidView,
            } else "plain";
            const code = style_codes.get(style) orelse return error.InvalidView;
            const take = @min(text.len, visible_end -| span_start);
            if (ansi and take != 0) try out.appendSlice(allocator, code);
            if (take != 0) try out.appendSlice(allocator, text[0..take]);
            span_start += text.len;
        }
        if (ansi) try out.appendSlice(allocator, "\x1b[0m");
        if (line_index + 1 < lines.len or final_newline) try out.appendSlice(allocator, "\n");
    }
    return lines.len;
}

pub fn cursorRow(view: std.json.Value, rows: usize) !usize {
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

pub fn renderForTest(allocator: std.mem.Allocator, view: std.json.Value, commit: bool) ![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    try appendScreenPrelude(&out, allocator);
    _ = try appendView(&out, allocator, view, 80, 24, true, commit);
    return out.toOwnedSlice(allocator);
}

test "one-column terminals saturate without underflow" {
    try std.testing.expectEqual(@as(usize, 0), usableColumns(0));
    try std.testing.expectEqual(@as(usize, 0), usableColumns(1));
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"byte\":0}}", .{});
    defer parsed.deinit();
    var out: std.ArrayList(u8) = .empty;
    defer out.deinit(std.testing.allocator);
    _ = try appendView(&out, std.testing.allocator, parsed.value, 1, 1, true, false);
    try std.testing.expect(std.mem.indexOf(u8, out.items, "x") == null);
    try std.testing.expect(std.mem.endsWith(u8, out.items, "\x1b[1;1H\x1b[?25h"));
}

test "presenter redraws the viewport absolutely and honors cursor" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"hello\",\"style\":\"accent\"}]},{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"byte\":2}}", .{});
    defer parsed.deinit();
    const frame = try renderForTest(std.testing.allocator, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.startsWith(u8, frame, "\x1b[?25l\x1b[H\x1b[2J"));
    try std.testing.expect(std.mem.endsWith(u8, frame, "\x1b[1;3H\x1b[?25h"));
    try std.testing.expect(std.mem.indexOf(u8, frame, "?1049") == null);
}

test "presenter converts semantic UTF-8 byte cursors across spans" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"> \"},{\"text\":\"aéz\"}]}],\"cursor\":{\"row\":1,\"byte\":5}}", .{});
    defer parsed.deinit();
    const frame = try renderForTest(std.testing.allocator, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.endsWith(u8, frame, "\x1b[1;5H\x1b[?25h"));
    var narrow: std.ArrayList(u8) = .empty;
    defer narrow.deinit(std.testing.allocator);
    _ = try appendView(&narrow, std.testing.allocator, parsed.value, 4, 24, true, false);
    try std.testing.expect(std.mem.endsWith(u8, narrow.items, "\x1b[1;3H\x1b[?25h"));

    var split = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"é\"}]}],\"cursor\":{\"row\":1,\"byte\":1}}", .{});
    defer split.deinit();
    try std.testing.expectError(error.InvalidView, renderForTest(std.testing.allocator, split.value, false));
}

test "graphemes cross styling spans and cursor cannot split them" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"e\",\"style\":\"bold\"},{\"text\":\"́x\",\"style\":\"accent\"}]}],\"cursor\":{\"row\":1,\"byte\":1}}", .{});
    defer parsed.deinit();
    try std.testing.expectError(error.InvalidView, renderForTest(std.testing.allocator, parsed.value, false));
    parsed.value.object.getPtr("cursor").?.object.getPtr("byte").?.* = .{ .integer = 3 };
    const frame = try renderForTest(std.testing.allocator, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.endsWith(u8, frame, "\x1b[1;2H\x1b[?25h"));

    var clipped = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"a\"},{\"text\":\"👩‍\"},{\"text\":\"💻z\"}]}]}", .{});
    defer clipped.deinit();
    var out: std.ArrayList(u8) = .empty;
    defer out.deinit(std.testing.allocator);
    _ = try appendView(&out, std.testing.allocator, clipped.value, 4, 24, false, false);
    try std.testing.expectEqualStrings("a👩‍💻", out.items);
}

test "cursor visibility and emoji clusters are explicit at the right edge" {
    var busy = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"busy\"}]}],\"cursor\":null}", .{});
    defer busy.deinit();
    const hidden = try renderForTest(std.testing.allocator, busy.value, false);
    defer std.testing.allocator.free(hidden);
    try std.testing.expect(std.mem.startsWith(u8, hidden, "\x1b[?25l"));
    try std.testing.expect(std.mem.indexOf(u8, hidden, "\x1b[?25h") == null);

    var emoji = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"a👩‍💻x\"}]}],\"cursor\":{\"row\":1,\"byte\":12}}", .{});
    defer emoji.deinit();
    var out: std.ArrayList(u8) = .empty;
    defer out.deinit(std.testing.allocator);
    _ = try appendView(&out, std.testing.allocator, emoji.value, 5, 24, true, false);
    try std.testing.expect(std.mem.endsWith(u8, out.items, "\x1b[1;4H\x1b[?25h"));
}
