//! JSON configuration parsing. This module validates only the harness shape;
//! the nested `config` value remains policy-free JSON.
const std = @import("std");

pub const Config = struct {
    parsed: std.json.Parsed(std.json.Value),
    extensions: []const std.json.Value,
    config_json: []u8,
    config_value: std.json.Value,
    owns_config_value: bool,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *Config) void {
        self.allocator.free(self.config_json);
        if (self.owns_config_value) self.config_value.object.deinit(self.allocator);
        self.parsed.deinit();
    }

    pub fn extensionPath(self: Config, index: usize) []const u8 {
        return self.extensions[index].string;
    }
};

pub fn parse(allocator: std.mem.Allocator, source: []const u8) !Config {
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source, .{});
    errdefer parsed.deinit();

    const root = switch (parsed.value) {
        .object => |object| object,
        else => return error.ConfigMustBeObject,
    };
    const extensions = if (root.get("extensions")) |extension_value| switch (extension_value) {
        .array => |array| array.items,
        else => return error.ExtensionsMustBeArray,
    } else &.{};
    for (extensions) |extension| {
        if (extension != .string) return error.ExtensionMustBeString;
        if (std.mem.indexOfScalar(u8, extension.string, 0) != null)
            return error.ExtensionContainsNul;
    }

    const has_config = root.get("config") != null;
    var config_value: std.json.Value = root.get("config") orelse .{ .object = .{} };
    errdefer if (!has_config) config_value.object.deinit(allocator);
    const config_json = try std.json.Stringify.valueAlloc(allocator, config_value, .{});
    return .{
        .parsed = parsed,
        .extensions = extensions,
        .config_json = config_json,
        .config_value = config_value,
        .owns_config_value = !has_config,
        .allocator = allocator,
    };
}

test "preserves extension order and serializes free-form config" {
    var config = try parse(std.testing.allocator,
        \\{"extensions":["first.lua","second.lua"],"config":{"visible":true,"n":3}}
    );
    defer config.deinit();
    try std.testing.expectEqualStrings("first.lua", config.extensionPath(0));
    try std.testing.expectEqualStrings("second.lua", config.extensionPath(1));
    try std.testing.expectEqualStrings("{\"visible\":true,\"n\":3}", config.config_json);
}

test "omitted fields use native defaults" {
    var config = try parse(std.testing.allocator, "{}");
    defer config.deinit();
    try std.testing.expectEqual(@as(usize, 0), config.extensions.len);
    try std.testing.expectEqualStrings("{}", config.config_json);

    var only_extensions = try parse(std.testing.allocator, "{\"extensions\":[]}");
    defer only_extensions.deinit();
    try std.testing.expectEqualStrings("{}", only_extensions.config_json);

    var only_config = try parse(std.testing.allocator, "{\"config\":[]}");
    defer only_config.deinit();
    try std.testing.expectEqual(@as(usize, 0), only_config.extensions.len);
    try std.testing.expectEqualStrings("[]", only_config.config_json);
}

test "rejects malformed harness shape" {
    try std.testing.expectError(error.ConfigMustBeObject, parse(std.testing.allocator, "[]"));
    try std.testing.expectError(error.ExtensionsMustBeArray, parse(std.testing.allocator, "{\"extensions\":{}}"));
    try std.testing.expectError(error.ExtensionMustBeString, parse(std.testing.allocator, "{\"extensions\":[1],\"config\":null}"));
    try std.testing.expectError(error.ExtensionContainsNul, parse(std.testing.allocator, "{\"extensions\":[\"bad\\u0000path.lua\"]}"));
}
