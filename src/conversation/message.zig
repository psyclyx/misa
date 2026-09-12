//! The canonical message shape the log accepts, and the tool-call pairing that
//! makes a recorded transcript replayable.
//!
//! Payload bytes stay opaque: the store never rewrites what it wrote, and a
//! schema migration never has to reinterpret them. What is *not* opaque is the
//! `message` kind's shape, because a transcript a provider rejects is not a
//! durable record of anything. Until this module existed, that shape was a
//! convention held by Lua, so a plugin could write history that failed only
//! later, at the next request.
//!
//! Pairing is checked in one direction only: a tool result must answer a call the
//! conversation has left open, and a call is answered at most once. A transcript
//! that *ends* with open calls is legal — that is what a crash mid-batch looks
//! like — and the fold closes it on load.

const std = @import("std");

pub const Role = enum { user, assistant, tool };

/// Validate one canonical message and report its role. Everything a provider
/// needs in order to accept the transcript is required here.
pub fn validate(data: std.json.Value) !Role {
    const object = switch (data) {
        .object => |value| value,
        else => return error.InvalidMessageShape,
    };
    const role_value = object.get("role") orelse return error.InvalidMessageShape;
    if (role_value != .string) return error.InvalidMessageShape;
    const content = object.get("content") orelse return error.InvalidMessageShape;
    if (content != .array) return error.InvalidMessageShape;
    for (content.array.items) |block| try validateBlock(block);

    if (std.mem.eql(u8, role_value.string, "user")) return .user;
    if (std.mem.eql(u8, role_value.string, "assistant")) return .assistant;
    if (std.mem.eql(u8, role_value.string, "tool")) {
        // A result without the call it answers, or without a verdict, is the
        // shape providers reject first.
        try requireNonEmptyString(object, "tool_call_id");
        const is_error = object.get("is_error") orelse return error.InvalidMessageShape;
        if (is_error != .bool) return error.InvalidMessageShape;
        return .tool;
    }
    return error.InvalidMessageShape;
}

/// The call ids an assistant message requests, borrowed from DATA. Validate the
/// message first.
pub fn toolCallIds(allocator: std.mem.Allocator, data: std.json.Value, out: *std.ArrayList([]const u8)) !void {
    for (data.object.get("content").?.array.items) |block| {
        if (!std.mem.eql(u8, block.object.get("type").?.string, "tool_call")) continue;
        try out.append(allocator, block.object.get("id").?.string);
    }
}

/// The call a tool result answers, borrowed from DATA. Validate the message
/// first.
pub fn toolResultId(data: std.json.Value) []const u8 {
    return data.object.get("tool_call_id").?.string;
}

fn validateBlock(block: std.json.Value) !void {
    const object = switch (block) {
        .object => |value| value,
        else => return error.InvalidMessageShape,
    };
    const kind_value = object.get("type") orelse return error.InvalidMessageShape;
    if (kind_value != .string) return error.InvalidMessageShape;
    const kind = kind_value.string;

    if (std.mem.eql(u8, kind, "text") or std.mem.eql(u8, kind, "thinking")) {
        const text = object.get("text") orelse return error.InvalidMessageShape;
        if (text != .string) return error.InvalidMessageShape;
        return;
    }
    if (std.mem.eql(u8, kind, "tool_call")) {
        try requireNonEmptyString(object, "id");
        try requireNonEmptyString(object, "name");
        const arguments = object.get("arguments") orelse return error.InvalidMessageShape;
        // An empty Lua table crosses the boundary as an empty array, so both
        // spell "this call takes no arguments"; anything else must be a mapping.
        switch (arguments) {
            .object => {},
            .array => |array| if (array.items.len != 0) return error.InvalidMessageShape,
            else => return error.InvalidMessageShape,
        }
        return;
    }
    if (std.mem.eql(u8, kind, "image")) {
        // The attachment an image tool call carried, in the shape the provider
        // protocols inject as base64.
        const source = object.get("source") orelse return error.InvalidMessageShape;
        if (source != .object) return error.InvalidMessageShape;
        const source_type = source.object.get("type") orelse return error.InvalidMessageShape;
        if (source_type != .string) return error.InvalidMessageShape;
        if (!std.mem.eql(u8, source_type.string, "base64")) return error.InvalidMessageShape;
        try requireNonEmptyString(source.object, "media_type");
        try requireNonEmptyString(source.object, "data");
        return;
    }
    return error.InvalidMessageShape;
}

fn requireNonEmptyString(object: std.json.ObjectMap, key: []const u8) !void {
    const value = object.get(key) orelse return error.InvalidMessageShape;
    if (value != .string) return error.InvalidMessageShape;
    if (value.string.len == 0) return error.InvalidMessageShape;
    if (std.mem.indexOfScalar(u8, value.string, 0) != null) return error.InvalidMessageShape;
}

fn parse(allocator: std.mem.Allocator, text: []const u8) !std.json.Parsed(std.json.Value) {
    return std.json.parseFromSlice(std.json.Value, allocator, text, .{ .allocate = .alloc_always });
}

test "a canonical message is accepted in every shape the session writes" {
    const a = std.testing.allocator;
    const accepted = [_][]const u8{
        "{\"role\":\"user\",\"content\":[]}",
        "{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}",
        "{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"media_type\":\"image/png\",\"data\":\"AA==\"}}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"text\":\"hmm\"},{\"type\":\"text\",\"text\":\"hi\"}],\"provider_state\":[{\"provider\":\"anthropic\",\"value\":{\"signature\":\"x\"}}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"call-1\",\"name\":\"read_file\",\"arguments\":{}}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"call-1\",\"name\":\"read_file\",\"arguments\":[]}]}",
        "{\"role\":\"tool\",\"content\":[{\"type\":\"text\",\"text\":\"contents\"}],\"is_error\":false,\"tool_call_id\":\"call-1\"}",
    };
    for (accepted) |text| {
        var parsed = try parse(a, text);
        defer parsed.deinit();
        _ = try validate(parsed.value);
    }
}

test "a message the provider would reject is refused at the boundary" {
    const a = std.testing.allocator;
    const refused = [_][]const u8{
        "[]",
        "{}",
        "{\"content\":[]}",
        "{\"role\":\"system\",\"content\":[]}",
        "{\"role\":\"user\"}",
        "{\"role\":\"user\",\"content\":{}}",
        "{\"role\":\"user\",\"content\":[\"hi\"]}",
        "{\"role\":\"user\",\"content\":[{\"type\":\"video\",\"text\":\"hi\"}]}",
        "{\"role\":\"user\",\"content\":[{\"type\":\"text\"}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"\",\"name\":\"x\",\"arguments\":{}}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"c\",\"name\":\"x\"}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"c\",\"name\":\"x\",\"arguments\":\"{}\"}]}",
        "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"c\",\"name\":\"x\",\"arguments\":[1]}]}",
        "{\"role\":\"tool\",\"content\":[{\"type\":\"text\",\"text\":\"x\"}]}",
        "{\"role\":\"tool\",\"content\":[{\"type\":\"text\",\"text\":\"x\"}],\"tool_call_id\":\"\",\"is_error\":false}",
        "{\"role\":\"tool\",\"content\":[{\"type\":\"text\",\"text\":\"x\"}],\"tool_call_id\":\"c\",\"is_error\":\"no\"}",
        "{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"url\",\"media_type\":\"image/png\",\"data\":\"AA==\"}}]}",
        "{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"media_type\":\"image/png\"}}]}",
    };
    for (refused) |text| {
        var parsed = try parse(a, text);
        defer parsed.deinit();
        try std.testing.expectError(error.InvalidMessageShape, validate(parsed.value));
    }
}

test "the call ids of an assistant message and the id of a result" {
    const a = std.testing.allocator;
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    var called = try parse(a, "{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_call\",\"id\":\"call-1\",\"name\":\"a\",\"arguments\":{}},{\"type\":\"text\",\"text\":\"also\"},{\"type\":\"tool_call\",\"id\":\"call-2\",\"name\":\"b\",\"arguments\":{}}]}");
    defer called.deinit();
    var ids: std.ArrayList([]const u8) = .empty;
    try toolCallIds(arena.allocator(), called.value, &ids);
    try std.testing.expectEqual(@as(usize, 2), ids.items.len);
    try std.testing.expectEqualStrings("call-1", ids.items[0]);
    try std.testing.expectEqualStrings("call-2", ids.items[1]);

    var result = try parse(a, "{\"role\":\"tool\",\"content\":[],\"is_error\":true,\"tool_call_id\":\"call-2\"}");
    defer result.deinit();
    try std.testing.expectEqualStrings("call-2", toolResultId(result.value));
}
