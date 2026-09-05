//! Native-only text entry for operation interactions. Published state contains
//! only character counts; secret bytes never become Lua events or database data.
const std = @import("std");
const terminal = @import("misa_terminal");

pub const Spec = struct { id: []const u8, correlation: []const u8, completion: []const u8 };
pub const Result = enum { changed, submit, cancel };
pub const capacity = 64 * 1024;

pub const Input = struct {
    allocator: std.mem.Allocator,
    spec: Spec,
    bytes: []u8,
    len: usize = 0,
    too_long: bool = false,

    pub fn init(allocator: std.mem.Allocator, spec: Spec) !Input {
        const id = try allocator.dupe(u8, spec.id);
        errdefer allocator.free(id);
        const correlation = try allocator.dupe(u8, spec.correlation);
        errdefer allocator.free(correlation);
        const completion = try allocator.dupe(u8, spec.completion);
        errdefer allocator.free(completion);
        return .{ .allocator = allocator, .spec = .{ .id = id, .correlation = correlation, .completion = completion }, .bytes = try allocator.alloc(u8, capacity) };
    }

    pub fn deinit(self: *Input) void {
        std.crypto.secureZero(u8, self.bytes);
        self.allocator.free(self.bytes);
        self.allocator.free(self.spec.id);
        self.allocator.free(self.spec.correlation);
        self.allocator.free(self.spec.completion);
    }

    pub fn text(self: *const Input) []const u8 {
        return self.bytes[0..self.len];
    }

    pub fn characters(self: *const Input) usize {
        var count: usize = 0;
        for (self.text()) |byte| if (byte & 0xc0 != 0x80) {
            count += 1;
        };
        return count;
    }

    pub fn accept(self: *Input, event: terminal.Event) Result {
        switch (event) {
            .text => |value| {
                var count: usize = 0;
                for (value) |byte| if (byte >= 0x20 and byte != 0x7f) {
                    count += 1;
                };
                if (count > capacity - self.len) {
                    self.too_long = true;
                    return .changed;
                }
                for (value) |byte| {
                    if (byte < 0x20 or byte == 0x7f) continue;
                    self.bytes[self.len] = byte;
                    self.len += 1;
                }
                self.too_long = false;
            },
            .backspace => if (self.len != 0) {
                const end = self.len;
                self.len -= 1;
                while (self.len > 0 and self.bytes[self.len] & 0xc0 == 0x80) self.len -= 1;
                std.crypto.secureZero(u8, self.bytes[self.len..end]);
                self.too_long = false;
            },
            .enter => return if (self.len == 0 or self.too_long) .changed else .submit,
            .escape, .ctrl_c, .ctrl_d, .eof => return .cancel,
            else => {},
        }
        return .changed;
    }
};

test "protected text is bounded, masks by character, and erases deleted bytes" {
    var input = try Input.init(std.testing.allocator, .{ .id = "op", .correlation = "secret", .completion = "input/changed" });
    defer input.deinit();
    try std.testing.expectEqual(Result.changed, input.accept(.enter));
    _ = input.accept(.{ .text = @constCast("key-é\n") });
    try std.testing.expectEqualStrings("key-é", input.text());
    try std.testing.expectEqual(Result.changed, input.accept(.{ .mouse = .{ .row = 1, .column = 1 } }));
    try std.testing.expectEqual(Result.changed, input.accept(.wheel_up));
    try std.testing.expectEqualStrings("key-é", input.text());
    try std.testing.expectEqual(@as(usize, 5), input.characters());
    _ = input.accept(.backspace);
    try std.testing.expectEqualStrings("key-", input.text());
    try std.testing.expectEqualSlices(u8, &.{ 0, 0 }, input.bytes[4..6]);
    try std.testing.expectEqual(Result.submit, input.accept(.enter));
    try std.testing.expectEqual(Result.cancel, input.accept(.escape));
    const large = try std.testing.allocator.alloc(u8, capacity);
    defer std.testing.allocator.free(large);
    @memset(large, 'x');
    _ = input.accept(.{ .text = large });
    try std.testing.expect(input.too_long);
    try std.testing.expectEqualStrings("key-", input.text());
    try std.testing.expectEqual(Result.changed, input.accept(.enter));
    _ = input.accept(.backspace);
    try std.testing.expect(!input.too_long);
    try std.testing.expectEqual(Result.submit, input.accept(.enter));
}
