//! Bounded bilinear thumbnails; no terminal geometry or transport policy.
const std = @import("std");
const Decoded = @import("decode.zig").Decoded;
pub fn render(allocator: std.mem.Allocator, original: Decoded) !Decoded {
    const scale = @min(@as(f64, 1), @min(@as(f64, 480) / @as(f64, @floatFromInt(original.width)), @as(f64, 320) / @as(f64, @floatFromInt(original.height))));
    const width: u32 = @max(1, @as(u32, @intFromFloat(@as(f64, @floatFromInt(original.width)) * scale)));
    const height: u32 = @max(1, @as(u32, @intFromFloat(@as(f64, @floatFromInt(original.height)) * scale)));
    const pixels = try allocator.alloc(u8, @as(usize, width) * height * 4);
    for (0..height) |y| {
        const sy = (@as(f64, @floatFromInt(y)) + 0.5) * @as(f64, @floatFromInt(original.height)) / @as(f64, @floatFromInt(height)) - 0.5;
        const y0: usize = @intFromFloat(@max(0, sy));
        const y1 = @min(y0 + 1, original.height - 1);
        const fy = @max(0, sy - @as(f64, @floatFromInt(y0)));
        for (0..width) |x| {
            const sx = (@as(f64, @floatFromInt(x)) + 0.5) * @as(f64, @floatFromInt(original.width)) / @as(f64, @floatFromInt(width)) - 0.5;
            const x0: usize = @intFromFloat(@max(0, sx));
            const x1 = @min(x0 + 1, original.width - 1);
            const fx = @max(0, sx - @as(f64, @floatFromInt(x0)));
            for (0..4) |channel| {
                const top = @as(f64, @floatFromInt(original.pixels[(y0 * original.width + x0) * 4 + channel])) * (1 - fx) + @as(f64, @floatFromInt(original.pixels[(y0 * original.width + x1) * 4 + channel])) * fx;
                const bottom = @as(f64, @floatFromInt(original.pixels[(y1 * original.width + x0) * 4 + channel])) * (1 - fx) + @as(f64, @floatFromInt(original.pixels[(y1 * original.width + x1) * 4 + channel])) * fx;
                pixels[(y * width + x) * 4 + channel] = @intFromFloat(@round(top * (1 - fy) + bottom * fy));
            }
        }
    }
    return .{ .pixels = pixels, .width = width, .height = height, .mime_type = "application/x-rgba" };
}

test "preview dimensions preserve aspect ratio" {
    const a = std.testing.allocator;
    const large = try a.alloc(u8, 1000 * 400 * 4);
    defer a.free(large);
    @memset(large, 128);
    const small = try render(a, .{ .pixels = large, .width = 1000, .height = 400, .mime_type = "image/png" });
    defer small.deinit(a);
    try std.testing.expectEqual(@as(u32, 480), small.width);
    try std.testing.expectEqual(@as(u32, 192), small.height);
    for (small.pixels) |pixel| try std.testing.expectEqual(@as(u8, 128), pixel);
}
