//! Catalog and path resolver for extensions shipped with misa.
const std = @import("std");
const build_options = @import("misa_build_options");

pub const default_config_path = build_options.default_config_path;

pub const ids = [_][]const u8{
    "agent",
    "provider.fake",
    "provider.command",
    "ui",
};

pub const ResolveError = error{
    UnknownStandardExtension,
    ExtensionPathContainsNul,
} || std.mem.Allocator.Error;

pub fn isLiteralPath(value: []const u8) bool {
    return std.mem.indexOfScalar(u8, value, '/') != null or std.mem.endsWith(u8, value, ".lua");
}

pub fn catalogPath(id: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, id, "agent")) return "agent.lua";
    if (std.mem.eql(u8, id, "provider.fake")) return "provider/fake.lua";
    if (std.mem.eql(u8, id, "provider.command")) return "provider/command.lua";
    if (std.mem.eql(u8, id, "ui")) return "ui.lua";
    return null;
}

/// The caller owns the returned path. Literal paths are copied unchanged.
pub fn resolve(
    allocator: std.mem.Allocator,
    value: []const u8,
    extension_dir: ?[]const u8,
) ResolveError![]u8 {
    // luaL_loadfile accepts a C string, so reject truncation at this boundary
    // even when callers did not obtain the value through config.parse.
    if (std.mem.indexOfScalar(u8, value, 0) != null) return error.ExtensionPathContainsNul;
    if (isLiteralPath(value)) return allocator.dupe(u8, value);
    const relative = catalogPath(value) orelse return error.UnknownStandardExtension;
    const root = extension_dir orelse build_options.default_extension_dir;
    return std.fs.path.join(allocator, &.{ root, relative });
}

test "catalog accepts exact IDs only" {
    try std.testing.expectEqualStrings("agent.lua", catalogPath("agent").?);
    try std.testing.expectEqualStrings("provider/fake.lua", catalogPath("provider.fake").?);
    try std.testing.expectEqualStrings("provider/command.lua", catalogPath("provider.command").?);
    try std.testing.expect(catalogPath("provider") == null);
    try std.testing.expect(catalogPath("Agent") == null);
    try std.testing.expectEqualStrings("ui.lua", catalogPath("ui").?);
    try std.testing.expectEqual(@as(usize, 4), ids.len);
}

test "resolver preserves literals and resolves catalog roots" {
    const allocator = std.testing.allocator;
    const literal = try resolve(allocator, "custom/x.lua", null);
    defer allocator.free(literal);
    try std.testing.expectEqualStrings("custom/x.lua", literal);

    const env_path = try resolve(allocator, "provider.fake", "/source/extensions");
    defer allocator.free(env_path);
    try std.testing.expectEqualStrings("/source/extensions/provider/fake.lua", env_path);

    const installed = try resolve(allocator, "agent", null);
    defer allocator.free(installed);
    try std.testing.expect(std.mem.endsWith(u8, installed, "/share/misa/extensions/agent.lua"));
    try std.testing.expectError(error.UnknownStandardExtension, resolve(allocator, "unknown", null));
    try std.testing.expectError(error.ExtensionPathContainsNul, resolve(allocator, "bad\x00.lua", null));
}
