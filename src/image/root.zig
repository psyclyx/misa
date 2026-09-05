//! Async image effect contract and immutable attachment projection. Codecs,
//! clipboard transport, and thumbnail resampling have separate owners.
const std = @import("std");
const codec = @import("decode.zig");
const clipboard = @import("clipboard.zig");
const thumbnail = @import("thumbnail.zig");

pub const max_bytes = codec.max_bytes;
pub const max_pixels = codec.max_pixels;
pub const Decoded = codec.Decoded;
pub const decode = codec.decode;

pub const Spec = struct {
    path: ?[]const u8 = null,
    argv: []const std.json.Value = &.{},
    id: []const u8,
    completion: []const u8,

    pub fn parse(kind: []const u8, object: std.json.ObjectMap) !Spec {
        var result: Spec = .{
            .id = try nonEmpty(object, "id"),
            .completion = try nonEmpty(object, "completion"),
        };
        if (std.mem.eql(u8, kind, "image/load")) {
            result.path = try nonEmpty(object, "path");
        } else if (std.mem.eql(u8, kind, "image/paste")) {
            if (object.get("argv")) |value| {
                if (value != .array or value.array.items.len == 0 or value.array.items.len > 64) return error.InvalidEffect;
                for (value.array.items, 0..) |arg, index| {
                    if (arg != .string or (index == 0 and arg.string.len == 0) or std.mem.indexOfScalar(u8, arg.string, 0) != null) return error.InvalidEffect;
                }
                result.argv = value.array.items;
            }
        } else return error.UnknownNativeEffect;
        return result;
    }
};

pub fn run(allocator: std.mem.Allocator, io: std.Io, spec: Spec, environ: *const std.process.Environ.Map) !std.json.Value {
    const bytes = if (spec.path) |path|
        try std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(max_bytes))
    else
        try clipboard.read(allocator, io, spec.argv, environ, max_bytes);
    defer allocator.free(bytes);
    const original = try decode(allocator, bytes);
    defer original.deinit(allocator);
    const preview = try thumbnail.render(allocator, original);
    defer preview.deinit(allocator);
    var result: std.json.ObjectMap = .empty;
    try result.put(allocator, "name", .{ .string = if (spec.path) |path| std.fs.path.basename(path) else "clipboard image" });
    try result.put(allocator, "mime_type", .{ .string = original.mime_type });
    try result.put(allocator, "data", .{ .string = try base64(allocator, bytes) });
    try result.put(allocator, "width", .{ .integer = original.width });
    try result.put(allocator, "height", .{ .integer = original.height });
    var small: std.json.ObjectMap = .empty;
    try small.put(allocator, "format", .{ .string = "rgba" });
    try small.put(allocator, "data", .{ .string = try base64(allocator, preview.pixels) });
    try small.put(allocator, "width", .{ .integer = preview.width });
    try small.put(allocator, "height", .{ .integer = preview.height });
    try result.put(allocator, "preview", .{ .object = small });
    return .{ .object = result };
}

fn base64(allocator: std.mem.Allocator, bytes: []const u8) ![]u8 {
    const encoder = std.base64.standard.Encoder;
    const result = try allocator.alloc(u8, encoder.calcSize(bytes.len));
    _ = encoder.encode(result, bytes);
    return result;
}

fn nonEmpty(object: std.json.ObjectMap, key: []const u8) ![]const u8 {
    const value = object.get(key) orelse return error.InvalidEffect;
    if (value != .string or value.string.len == 0 or std.mem.indexOfScalar(u8, value.string, 0) != null) return error.InvalidEffect;
    return value.string;
}

test {
    _ = @import("decode.zig");
    _ = @import("thumbnail.zig");
    _ = @import("clipboard.zig");
}
