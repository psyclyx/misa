//! Decode complete buffered SSE and JSONL bodies into one owned JSON array.
const std = @import("std");

const max_record_bytes = 256 * 1024;

pub fn parseSseJson(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(std.json.Value) {
    var document: std.ArrayList(u8) = .empty;
    defer document.deinit(allocator);
    var record: std.ArrayList(u8) = .empty;
    defer record.deinit(allocator);
    try document.append(allocator, '[');
    var first = true;
    var remaining = source;
    while (std.mem.indexOfScalar(u8, remaining, '\n')) |newline| {
        const line = std.mem.trimEnd(u8, remaining[0..newline], "\r");
        remaining = remaining[newline + 1 ..];
        if (line.len == 0) {
            try appendSseRecord(allocator, &document, &record, &first);
            continue;
        }
        if (!std.mem.startsWith(u8, line, "data:")) continue;
        const data = std.mem.trimStart(u8, line[5..], " \t");
        if (record.items.len != 0) try record.append(allocator, '\n');
        try record.appendSlice(allocator, data);
        if (record.items.len > max_record_bytes) return error.StreamRecordTooLarge;
    }
    // EOF is not an SSE dispatch boundary, so an unterminated line or record
    // is deliberately omitted.
    try document.append(allocator, ']');
    return std.json.parseFromSlice(std.json.Value, allocator, document.items, .{ .allocate = .alloc_always });
}

fn appendSseRecord(allocator: std.mem.Allocator, document: *std.ArrayList(u8), record: *std.ArrayList(u8), first: *bool) !void {
    const data = std.mem.trim(u8, record.items, " \t\r\n");
    if (data.len != 0 and !std.mem.eql(u8, data, "[DONE]")) {
        if (!first.*) try document.append(allocator, ',');
        first.* = false;
        try document.appendSlice(allocator, data);
    }
    record.clearRetainingCapacity();
}

pub fn parseJsonLines(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(std.json.Value) {
    var document: std.ArrayList(u8) = .empty;
    defer document.deinit(allocator);
    try document.append(allocator, '[');
    var lines = std.mem.splitScalar(u8, source, '\n');
    var first = true;
    while (lines.next()) |raw| {
        const line = std.mem.trim(u8, raw, " \t\r");
        if (line.len == 0) continue;
        if (line.len > max_record_bytes) return error.StreamRecordTooLarge;
        if (!first) try document.append(allocator, ',');
        first = false;
        try document.appendSlice(allocator, line);
    }
    try document.append(allocator, ']');
    return std.json.parseFromSlice(std.json.Value, allocator, document.items, .{ .allocate = .alloc_always });
}

test "SSE data records become one owned array" {
    var parsed = try parseSseJson(std.testing.allocator, "event: update\ndata: {\"type\":\ndata: \"delta\"}\n\ndata: [DONE]\n");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(usize, 1), parsed.value.array.items.len);
    try std.testing.expectEqualStrings("delta", parsed.value.array.items[0].object.get("type").?.string);
}

test "SSE EOF drops unterminated records" {
    var parsed = try parseSseJson(std.testing.allocator, "data: {\"complete\":true}\n\ndata: {\"partial\":true}\n");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(usize, 1), parsed.value.array.items.len);
    try std.testing.expect(parsed.value.array.items[0].object.get("complete").?.bool);
}

test "JSON lines become one owned array" {
    var parsed = try parseJsonLines(std.testing.allocator, "{\"type\":\"start\"}\n\n{\"type\":\"result\",\"text\":\"ok\"}\n");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(usize, 2), parsed.value.array.items.len);
    try std.testing.expectEqualStrings("ok", parsed.value.array.items[1].object.get("text").?.string);
}
