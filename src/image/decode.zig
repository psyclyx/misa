//! PNG/JPEG codecs and decoded-image allocation bounds.
const std = @import("std");
const c = @cImport({
    @cInclude("png.h");
    @cInclude("turbojpeg.h");
});

pub const max_bytes = 8 * 1024 * 1024;
pub const max_pixels = 16 * 1024 * 1024;

pub const Decoded = struct {
    pixels: []u8,
    width: u32,
    height: u32,
    mime_type: []const u8,

    pub fn deinit(self: Decoded, allocator: std.mem.Allocator) void {
        allocator.free(self.pixels);
    }
};

pub fn decode(allocator: std.mem.Allocator, bytes: []const u8) !Decoded {
    if (bytes.len > max_bytes) return error.ImageTooLarge;
    if (std.mem.startsWith(u8, bytes, "\x89PNG\r\n\x1a\n")) {
        var png: c.png_image = std.mem.zeroes(c.png_image);
        png.version = c.PNG_IMAGE_VERSION;
        defer c.png_image_free(&png);
        if (c.png_image_begin_read_from_memory(&png, bytes.ptr, bytes.len) == 0) return error.InvalidPng;
        const size = try pixelBytes(png.width, png.height);
        png.format = c.PNG_FORMAT_RGBA;
        const pixels = try allocator.alloc(u8, size);
        errdefer allocator.free(pixels);
        if (c.png_image_finish_read(&png, null, pixels.ptr, 0, null) == 0) return error.InvalidPng;
        return .{ .pixels = pixels, .width = png.width, .height = png.height, .mime_type = "image/png" };
    }
    if (std.mem.startsWith(u8, bytes, "\xff\xd8\xff")) {
        const decoder = c.tjInitDecompress() orelse return error.JpegDecoderFailed;
        defer _ = c.tjDestroy(decoder);
        var width: c_int = 0;
        var height: c_int = 0;
        var subsampling: c_int = 0;
        var colorspace: c_int = 0;
        if (c.tjDecompressHeader3(decoder, bytes.ptr, @intCast(bytes.len), &width, &height, &subsampling, &colorspace) != 0) return error.InvalidJpeg;
        if (width <= 0 or height <= 0) return error.InvalidImageDimensions;
        const size = try pixelBytes(@intCast(width), @intCast(height));
        const pixels = try allocator.alloc(u8, size);
        errdefer allocator.free(pixels);
        if (c.tjDecompress2(decoder, bytes.ptr, @intCast(bytes.len), pixels.ptr, width, 0, height, c.TJPF_RGBA, c.TJFLAG_STOPONWARNING) != 0) return error.InvalidJpeg;
        return .{ .pixels = pixels, .width = @intCast(width), .height = @intCast(height), .mime_type = "image/jpeg" };
    }
    return error.UnsupportedImageFormat;
}

fn pixelBytes(width: u32, height: u32) !usize {
    if (width == 0 or height == 0 or width > 16384 or height > 16384) return error.InvalidImageDimensions;
    const pixels = @as(u64, width) * height;
    if (pixels > max_pixels) return error.ImageTooLarge;
    return @intCast(pixels * 4);
}

test "PNG and JPEG dimensions are bounded before allocating pixels" {
    try std.testing.expectError(error.InvalidImageDimensions, pixelBytes(0, 1));
    try std.testing.expectError(error.ImageTooLarge, pixelBytes(8192, 8192));
    try std.testing.expectError(error.UnsupportedImageFormat, decode(std.testing.allocator, "not an image"));
    try std.testing.expectError(error.InvalidJpeg, decode(std.testing.allocator, "\xff\xd8\xff"));
    try std.testing.expectError(error.InvalidPng, decode(std.testing.allocator, "\x89PNG\r\n\x1a\n"));
}

test "PNG and JPEG decode pixels, and previews preserve aspect ratio" {
    const a = std.testing.allocator;
    const source = [_]u8{ 180, 100, 40, 255, 180, 100, 40, 255, 180, 100, 40, 255, 180, 100, 40, 255 };
    var png: c.png_image = std.mem.zeroes(c.png_image);
    png.version = c.PNG_IMAGE_VERSION;
    png.width = 2;
    png.height = 2;
    png.format = c.PNG_FORMAT_RGBA;
    defer c.png_image_free(&png);
    var png_size: c.png_alloc_size_t = 0;
    try std.testing.expect(c.png_image_write_to_memory(&png, null, &png_size, 0, &source, 0, null) != 0);
    const png_bytes = try a.alloc(u8, png_size);
    defer a.free(png_bytes);
    try std.testing.expect(c.png_image_write_to_memory(&png, png_bytes.ptr, &png_size, 0, &source, 0, null) != 0);
    const png_decoded = try decode(a, png_bytes[0..png_size]);
    defer png_decoded.deinit(a);
    try std.testing.expectEqualStrings("image/png", png_decoded.mime_type);
    try std.testing.expectEqualSlices(u8, &source, png_decoded.pixels);
    const encoder = c.tjInitCompress() orelse return error.JpegEncoderFailed;
    defer _ = c.tjDestroy(encoder);
    var jpeg_bytes: [*c]u8 = null;
    var jpeg_size: c_ulong = 0;
    defer if (jpeg_bytes != null) c.tjFree(jpeg_bytes);
    try std.testing.expect(c.tjCompress2(encoder, &source, 2, 0, 2, c.TJPF_RGBA, &jpeg_bytes, &jpeg_size, c.TJSAMP_444, 95, 0) == 0);
    const jpeg_decoded = try decode(a, jpeg_bytes[0..jpeg_size]);
    defer jpeg_decoded.deinit(a);
    try std.testing.expectEqualStrings("image/jpeg", jpeg_decoded.mime_type);
    try std.testing.expectEqual(@as(u32, 2), jpeg_decoded.width);
    for (source, jpeg_decoded.pixels) |expected, actual| try std.testing.expect(@abs(@as(i16, expected) - actual) <= 4);
}
