//! Catalog and path resolver for extensions shipped with misa.
const std = @import("std");
const build_options = @import("misa_build_options");

pub const default_config_path = build_options.default_config_path;
pub const default_extension_dir = build_options.default_extension_dir;

pub const Entry = struct { id: []const u8, path: []const u8 };

/// Single source for discovery, build-time translation, and runtime resolution.
pub const entries = [_]Entry{
    .{ .id = "misa.agent", .path = "misa/agent/init.fnl" },
    .{ .id = "misa.agent.costs", .path = "misa/agent/costs.fnl" },
    .{ .id = "misa.agent.effort", .path = "misa/agent/effort.fnl" },
    .{ .id = "misa.agent.models", .path = "misa/agent/models.fnl" },
    .{ .id = "misa.agent.request-options", .path = "misa/agent/request-options.fnl" },
    .{ .id = "misa.choices", .path = "misa/choices/init.fnl" },
    .{ .id = "misa.choices.layout", .path = "misa/choices/layout.fnl" },
    .{ .id = "misa.choices.picker", .path = "misa/choices/picker/init.fnl" },
    .{ .id = "misa.choices.picker.view", .path = "misa/choices/picker/view.fnl" },
    .{ .id = "misa.choices.preferences", .path = "misa/choices/preferences.fnl" },
    .{ .id = "misa.choices.preview", .path = "misa/choices/preview.fnl" },
    .{ .id = "misa.commands", .path = "misa/commands/init.fnl" },
    .{ .id = "misa.commands.actions", .path = "misa/commands/actions.fnl" },
    .{ .id = "misa.commands.keybindings", .path = "misa/commands/keybindings.fnl" },
    .{ .id = "misa.commands.palette", .path = "misa/commands/palette.fnl" },
    .{ .id = "misa.default", .path = "misa/default.fnl" },
    .{ .id = "misa.definitions", .path = "misa/definitions.fnl" },
    .{ .id = "misa.dialogs", .path = "misa/dialogs/init.fnl" },
    .{ .id = "misa.dialogs.view", .path = "misa/dialogs/view.fnl" },
    .{ .id = "misa.editor", .path = "misa/editor/init.fnl" },
    .{ .id = "misa.editor.attachments", .path = "misa/editor/attachments.fnl" },
    .{ .id = "misa.editor.editing", .path = "misa/editor/editing.fnl" },
    .{ .id = "misa.editor.history", .path = "misa/editor/history.fnl" },
    .{ .id = "misa.editor.images", .path = "misa/editor/images.fnl" },
    .{ .id = "misa.editor.queue", .path = "misa/editor/queue/init.fnl" },
    .{ .id = "misa.editor.queue.view", .path = "misa/editor/queue/view.fnl" },
    .{ .id = "misa.json", .path = "misa/json.fnl" },
    .{ .id = "misa.protocols.anthropic", .path = "misa/protocols/anthropic.fnl" },
    .{ .id = "misa.protocols.openai", .path = "misa/protocols/openai.fnl" },
    .{ .id = "misa.protocols.stream", .path = "misa/protocols/stream.fnl" },
    .{ .id = "misa.providers.anthropic", .path = "misa/providers/anthropic.fnl" },
    .{ .id = "misa.providers.auth", .path = "misa/providers/auth.fnl" },
    .{ .id = "misa.providers.claude", .path = "misa/providers/claude.fnl" },
    .{ .id = "misa.providers.command", .path = "misa/providers/command.fnl" },
    .{ .id = "misa.providers.fake", .path = "misa/providers/fake.fnl" },
    .{ .id = "misa.providers.kimi", .path = "misa/providers/kimi.fnl" },
    .{ .id = "misa.providers.openai", .path = "misa/providers/openai.fnl" },
    .{ .id = "misa.providers.openai-codex", .path = "misa/providers/openai-codex.fnl" },
    .{ .id = "misa.providers.openrouter", .path = "misa/providers/openrouter.fnl" },
    .{ .id = "misa.selection", .path = "misa/selection/init.fnl" },
    .{ .id = "misa.selection.document", .path = "misa/selection/document.fnl" },
    .{ .id = "misa.standard", .path = "misa/standard.fnl" },
    .{ .id = "misa.system.clipboard", .path = "misa/system/clipboard.fnl" },
    .{ .id = "misa.system.links", .path = "misa/system/links.fnl" },
    .{ .id = "misa.text.fuzzy", .path = "misa/text/fuzzy.fnl" },
    .{ .id = "misa.text.markdown", .path = "misa/text/markdown.fnl" },
    .{ .id = "misa.text.syntax", .path = "misa/text/syntax.fnl" },
    .{ .id = "misa.tools.files", .path = "misa/tools/files.fnl" },
    .{ .id = "misa.tools.shell", .path = "misa/tools/shell.fnl" },
    .{ .id = "misa.tools.summary", .path = "misa/tools/summary.fnl" },
    .{ .id = "misa.ui", .path = "misa/ui/init.fnl" },
    .{ .id = "misa.ui.animations", .path = "misa/ui/animations/init.fnl" },
    .{ .id = "misa.ui.animations.default", .path = "misa/ui/animations/default.fnl" },
    .{ .id = "misa.ui.components", .path = "misa/ui/components/init.fnl" },
    .{ .id = "misa.ui.components.buttons", .path = "misa/ui/components/buttons.fnl" },
    .{ .id = "misa.ui.components.chrome", .path = "misa/ui/components/chrome.fnl" },
    .{ .id = "misa.ui.components.content", .path = "misa/ui/components/content.fnl" },
    .{ .id = "misa.ui.components.data", .path = "misa/ui/components/data.fnl" },
    .{ .id = "misa.ui.components.dialog", .path = "misa/ui/components/dialog.fnl" },
    .{ .id = "misa.ui.components.editor", .path = "misa/ui/components/editor.fnl" },
    .{ .id = "misa.ui.components.group", .path = "misa/ui/components/group.fnl" },
    .{ .id = "misa.ui.components.image", .path = "misa/ui/components/image.fnl" },
    .{ .id = "misa.ui.components.markdown", .path = "misa/ui/components/markdown.fnl" },
    .{ .id = "misa.ui.components.message", .path = "misa/ui/components/message.fnl" },
    .{ .id = "misa.ui.components.picker", .path = "misa/ui/components/picker.fnl" },
    .{ .id = "misa.ui.components.selection", .path = "misa/ui/components/selection.fnl" },
    .{ .id = "misa.ui.components.status", .path = "misa/ui/components/status.fnl" },
    .{ .id = "misa.ui.components.tool", .path = "misa/ui/components/tool.fnl" },
    .{ .id = "misa.ui.components.truncation", .path = "misa/ui/components/truncation.fnl" },
    .{ .id = "misa.ui.layout", .path = "misa/ui/layout.fnl" },
    .{ .id = "misa.ui.status", .path = "misa/ui/status/init.fnl" },
    .{ .id = "misa.ui.status.indicators", .path = "misa/ui/status/indicators.fnl" },
    .{ .id = "misa.ui.status.usage", .path = "misa/ui/status/usage.fnl" },
    .{ .id = "misa.ui.themes", .path = "misa/ui/themes/init.fnl" },
    .{ .id = "misa.ui.themes.default", .path = "misa/ui/themes/default.fnl" },
    .{ .id = "misa.ui.tools", .path = "misa/ui/tools.fnl" },
    .{ .id = "misa.ui.transcript", .path = "misa/ui/transcript.fnl" },
    .{ .id = "misa.ui.values", .path = "misa/ui/values.fnl" },
};

pub const ids = blk: {
    var result: [entries.len][]const u8 = undefined;
    for (entries, 0..) |entry, index| result[index] = entry.id;
    break :blk result;
};

pub const ResolveError = error{
    UnknownStandardExtension,
    ExtensionPathContainsNul,
} || std.mem.Allocator.Error;

pub fn isLiteralPath(value: []const u8) bool {
    return std.mem.indexOfScalar(u8, value, '/') != null or std.mem.endsWith(u8, value, ".fnl") or std.mem.endsWith(u8, value, ".lua");
}

pub fn catalogPath(id: []const u8) ?[]const u8 {
    for (entries) |entry| {
        if (std.mem.eql(u8, id, entry.id)) return entry.path;
    }
    return null;
}

/// The caller owns the returned path. Literal paths are copied unchanged.
pub fn resolve(
    allocator: std.mem.Allocator,
    value: []const u8,
    extension_dir: ?[]const u8,
) ResolveError![]u8 {
    // luaL_loadfile accepts a C string; embedded NUL would truncate the path.
    if (std.mem.indexOfScalar(u8, value, 0) != null) return error.ExtensionPathContainsNul;
    if (isLiteralPath(value)) return allocator.dupe(u8, value);
    const relative = catalogPath(value) orelse return error.UnknownStandardExtension;
    const root = extension_dir orelse build_options.default_extension_dir;
    if (extension_dir != null) return std.fs.path.join(allocator, &.{ root, relative });
    const compiled = try std.fmt.allocPrint(allocator, "{s}.lua", .{relative[0 .. relative.len - 4]});
    defer allocator.free(compiled);
    return std.fs.path.join(allocator, &.{ root, compiled });
}

test "catalog paths mirror sorted namespaced IDs" {
    try std.testing.expect(entries.len > 0);
    for (entries, 0..) |entry, index| {
        try std.testing.expectEqualStrings(entry.id, ids[index]);
        try std.testing.expectEqualStrings(entry.path, catalogPath(entry.id).?);
        try std.testing.expect(std.mem.startsWith(u8, entry.id, "misa."));
        if (index > 0) try std.testing.expect(std.mem.order(u8, entries[index - 1].id, entry.id) == .lt);
        const stem = if (std.mem.endsWith(u8, entry.path, "/init.fnl"))
            entry.path[0 .. entry.path.len - "/init.fnl".len]
        else
            entry.path[0 .. entry.path.len - ".fnl".len];
        try std.testing.expectEqual(entry.id.len, stem.len);
        for (entry.id, stem) |name_byte, path_byte| {
            try std.testing.expectEqual(if (name_byte == '.') @as(u8, '/') else name_byte, path_byte);
        }
        try std.testing.expect(!isLiteralPath(entry.id));
        try std.testing.expect(!std.fs.path.isAbsolute(entry.path));
        try std.testing.expect(std.mem.endsWith(u8, entry.path, ".fnl"));
        var parts = std.mem.splitScalar(u8, entry.path, '/');
        while (parts.next()) |part| {
            try std.testing.expect(part.len > 0);
            try std.testing.expect(!std.mem.eql(u8, part, ".."));
            try std.testing.expect(!std.mem.eql(u8, part, "."));
        }
        for (entries[0..index]) |previous| {
            try std.testing.expect(!std.mem.eql(u8, previous.id, entry.id));
            try std.testing.expect(!std.mem.eql(u8, previous.path, entry.path));
        }
    }
}

test "catalog accepts namespaced leaf and owner IDs only" {
    try std.testing.expectEqualStrings("misa/agent/init.fnl", catalogPath("misa.agent").?);
    try std.testing.expectEqualStrings("misa/choices/layout.fnl", catalogPath("misa.choices.layout").?);
    try std.testing.expectEqualStrings("misa/ui/components/init.fnl", catalogPath("misa.ui.components").?);
    try std.testing.expectEqualStrings("misa/ui/components/markdown.fnl", catalogPath("misa.ui.components.markdown").?);
    for ([_][]const u8{ "agent", "component.markdown", "misa.providers", "misa.Agent", "misa.agent.init" }) |id| {
        try std.testing.expect(catalogPath(id) == null);
    }
}

test "resolver preserves literals and resolves catalog roots" {
    const allocator = std.testing.allocator;
    const literal = try resolve(allocator, "custom/x.fnl", null);
    defer allocator.free(literal);
    try std.testing.expectEqualStrings("custom/x.fnl", literal);

    const env_path = try resolve(allocator, "misa.providers.fake", "/source/extensions");
    defer allocator.free(env_path);
    try std.testing.expectEqualStrings("/source/extensions/misa/providers/fake.fnl", env_path);

    const installed = try resolve(allocator, "misa.agent", null);
    defer allocator.free(installed);
    try std.testing.expect(std.mem.endsWith(u8, installed, "/share/misa/extensions/misa/agent/init.lua"));
    try std.testing.expectError(error.UnknownStandardExtension, resolve(allocator, "unknown", null));
    try std.testing.expectError(error.ExtensionPathContainsNul, resolve(allocator, "bad\x00.fnl", null));
}

test "resolver preserves legacy Lua extension paths" {
    const path = try resolve(std.testing.allocator, "custom.lua", null);
    defer std.testing.allocator.free(path);
    try std.testing.expectEqualStrings("custom.lua", path);
}
