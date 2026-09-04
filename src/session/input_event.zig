//! Serialize decoded terminal input into the session event contract.
const std = @import("std");
const terminal = @import("misa_terminal");

pub fn serialize(allocator: std.mem.Allocator, event: terminal.Event) ![]u8 {
    return switch (event) {
        .text => |text| std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = "text", .text = text }, .{}),
        .alt => |text| std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = "alt", .text = text }, .{}),
        .enter => serializeKind(allocator, "enter"),
        .backspace => serializeKind(allocator, "backspace"),
        .tab => serializeKind(allocator, "tab"),
        .arrow_up => serializeKind(allocator, "arrow_up"),
        .arrow_down => serializeKind(allocator, "arrow_down"),
        .arrow_left => serializeKind(allocator, "arrow_left"),
        .arrow_right => serializeKind(allocator, "arrow_right"),
        .page_up => serializeKind(allocator, "page_up"),
        .page_down => serializeKind(allocator, "page_down"),
        .escape => serializeKind(allocator, "escape"),
        .ctrl_c => serializeKind(allocator, "ctrl_c"),
        .ctrl_d => serializeKind(allocator, "ctrl_d"),
        .eof => serializeKind(allocator, "eof"),
    };
}

fn serializeKind(allocator: std.mem.Allocator, kind: []const u8) ![]u8 {
    return std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = kind }, .{});
}

test "Alt input extends the terminal event schema" {
    const alt = try serialize(std.testing.allocator, .{ .alt = @constCast("3") });
    defer std.testing.allocator.free(alt);
    try std.testing.expectEqualStrings("{\"type\":\"terminal/input\",\"kind\":\"alt\",\"text\":\"3\"}", alt);

    const enter = try serialize(std.testing.allocator, .enter);
    defer std.testing.allocator.free(enter);
    try std.testing.expectEqualStrings("{\"type\":\"terminal/input\",\"kind\":\"enter\"}", enter);
}
