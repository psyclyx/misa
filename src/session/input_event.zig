//! Serialize decoded terminal input into the session event contract.
const std = @import("std");
const terminal = @import("misa_terminal");

pub fn serialize(allocator: std.mem.Allocator, event: terminal.Event) ![]u8 {
    return switch (event) {
        .text => |text| std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = "text", .text = text }, .{}),
        .alt => |text| std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = "alt", .text = text }, .{}),
        .enter => serializeKind(allocator, "enter"),
        .shift_enter => serializeKind(allocator, "shift_enter"),
        .alt_enter => serializeKind(allocator, "alt_enter"),
        .ctrl_p => serializeKind(allocator, "ctrl_p"),
        .ctrl_n => serializeKind(allocator, "ctrl_n"),
        .ctrl_v => serializeKind(allocator, "ctrl_v"),
        .backspace => serializeKind(allocator, "backspace"),
        .tab => serializeKind(allocator, "tab"),
        .f1 => serializeKind(allocator, "f1"),
        .arrow_up => serializeKind(allocator, "arrow_up"),
        .arrow_down => serializeKind(allocator, "arrow_down"),
        .arrow_left => serializeKind(allocator, "arrow_left"),
        .arrow_right => serializeKind(allocator, "arrow_right"),
        .page_up => serializeKind(allocator, "page_up"),
        .page_down => serializeKind(allocator, "page_down"),
        .wheel_up => serializeKind(allocator, "wheel_up"),
        .wheel_down => serializeKind(allocator, "wheel_down"),
        .mouse => |position| std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = "mouse", .row = position.row, .column = position.column }, .{}),
        .mouse_move => |position| std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = "mouse_move", .row = position.row, .column = position.column }, .{}),
        .escape => serializeKind(allocator, "escape"),
        .ctrl_c => serializeKind(allocator, "ctrl_c"),
        .ctrl_d => serializeKind(allocator, "ctrl_d"),
        .ctrl_r => serializeKind(allocator, "ctrl_r"),
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
