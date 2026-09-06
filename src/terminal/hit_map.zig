//! Semantic actions resolved in terminal cells, retained only for shown frames.
const std = @import("std");
const width = @import("width.zig");
const Hit = struct { row: usize, first: usize, end: usize, action: []u8 };
pub const Map = struct {
    hits: std.ArrayList(Hit) = .empty,
    pub fn deinit(self: *Map, allocator: std.mem.Allocator) void {
        for (self.hits.items) |hit| allocator.free(hit.action);
        self.hits.deinit(allocator);
        self.* = .{};
    }
    pub fn at(self: *const Map, row: usize, column: usize) ?[]const u8 {
        for (self.hits.items) |hit| if (hit.row == row and column >= hit.first and column < hit.end) return hit.action;
        return null;
    }
    pub fn clone(self: *const Map, allocator: std.mem.Allocator) !Map {
        var result: Map = .{};
        errdefer result.deinit(allocator);
        for (self.hits.items) |hit| {
            const action = try allocator.dupe(u8, hit.action);
            errdefer allocator.free(action);
            try result.hits.append(allocator, .{ .row = hit.row, .first = hit.first, .end = hit.end, .action = action });
        }
        return result;
    }
};

pub fn validateAction(value: ?std.json.Value) !void {
    const action = value orelse return;
    if (action != .string or action.string.len == 0 or action.string.len > 4096) return error.InvalidView;
    for (action.string) |byte| if (byte < 32 or byte == 127) return error.InvalidView;
}

pub fn build(allocator: std.mem.Allocator, view: std.json.Value, columns: usize) !Map {
    var result: Map = .{};
    errdefer result.deinit(allocator);
    const lines = view.object.get("lines").?.array.items; // presenter validated the view
    for (lines, 1..) |line, row| {
        const spans = line.object.get("spans").?.array.items;
        var text: std.ArrayList(u8) = .empty;
        defer text.deinit(allocator);
        for (spans) |span| {
            try text.appendSlice(allocator, span.object.get("text").?.string);
            try validateAction(span.object.get("action"));
        }
        var offset: usize = 0;
        var column: usize = 1;
        var span_index: usize = 0;
        var span_end: usize = if (spans.len > 0) spans[0].object.get("text").?.string.len else 0;
        while (offset < text.items.len) {
            const cluster = try width.nextCluster(text.items, offset);
            if (column - 1 + cluster.width > columns) break;
            while (offset >= span_end and span_index + 1 < spans.len) {
                span_index += 1;
                span_end += spans[span_index].object.get("text").?.string.len;
            }
            if (spans[span_index].object.get("action")) |action| {
                if (cluster.width > 0) {
                    const previous: ?*Hit = if (result.hits.items.len > 0) &result.hits.items[result.hits.items.len - 1] else null;
                    if (previous != null and previous.?.row == row and previous.?.end == column and std.mem.eql(u8, previous.?.action, action.string)) {
                        previous.?.end += cluster.width;
                    } else {
                        const id = try allocator.dupe(u8, action.string);
                        errdefer allocator.free(id);
                        try result.hits.append(allocator, .{ .row = row, .first = column, .end = column + cluster.width, .action = id });
                    }
                }
            }
            column += cluster.width;
            offset = cluster.end;
        }
    }
    return result;
}

test "hit map uses displayed grapheme cells and clips invisible actions" {
    const allocator = std.testing.allocator;
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, "{\"lines\":[{\"spans\":[{\"text\":\"界\",\"action\":\"wide\"},{\"text\":\"e\",\"action\":\"accent\"},{\"text\":\"́\"},{\"text\":\"hidden\",\"action\":\"clipped\"}]}]}", .{});
    defer parsed.deinit();
    var map = try build(allocator, parsed.value, 3);
    defer map.deinit(allocator);
    try std.testing.expectEqualStrings("wide", map.at(1, 1).?);
    try std.testing.expectEqualStrings("wide", map.at(1, 2).?);
    try std.testing.expectEqualStrings("accent", map.at(1, 3).?);
    try std.testing.expect(map.at(1, 4) == null);
}
