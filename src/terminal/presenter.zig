//! Scrollback-safe semantic frame presentation.
const std = @import("std");
const cell_width = @import("width.zig");

pub fn appendErase(out: *std.ArrayList(u8), allocator: std.mem.Allocator, rows: usize, cursor_row: usize) !void {
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

pub fn appendView(out: *std.ArrayList(u8), allocator: std.mem.Allocator, view: std.json.Value, columns: usize, max_lines: usize, ansi: bool, final_newline: bool) !usize {
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
    return .{ .row = row, .column = @min(width +| 1, @max(@as(usize, 1), columns -| 1)) };
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
            width += cell_width.displayWidth(std.unicode.utf8Decode(encoded) catch return error.InvalidView);
            offset += encoded.len;
        }
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
                const char_width = cell_width.displayWidth(cp);
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

pub fn renderForTest(allocator: std.mem.Allocator, previous_rows: usize, view: std.json.Value, commit: bool) ![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    try appendErase(&out, allocator, previous_rows, previous_rows);
    _ = try appendView(&out, allocator, view, 80, 24, true, commit);
    return out.toOwnedSlice(allocator);
}

test "presenter erases exact rows, honors cursor, and forbids screen clears" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"lines\":[{\"spans\":[{\"text\":\"hello\",\"style\":\"accent\"}]},{\"spans\":[{\"text\":\"x\"}]}],\"cursor\":{\"row\":1,\"byte\":2}}", .{});
    defer parsed.deinit();
    const frame = try renderForTest(std.testing.allocator, 2, parsed.value, false);
    defer std.testing.allocator.free(frame);
    try std.testing.expect(std.mem.startsWith(u8, frame, "\r\x1b[2K\x1b[1A\r\x1b[2K"));
    try std.testing.expect(std.mem.endsWith(u8, frame, "\x1b[1A\r\x1b[2C"));
    for ([_][]const u8{ "\x1b[J", "\x1b[2J", "\x1b[3J", "?1049" }) |forbidden| try std.testing.expect(std.mem.indexOf(u8, frame, forbidden) == null);
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
