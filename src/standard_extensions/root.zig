//! Catalog and path resolver for extensions shipped with misa.
const std = @import("std");
const build_options = @import("misa_build_options");

pub const default_config_path = build_options.default_config_path;

pub const Entry = struct { id: []const u8, path: []const u8 };

/// Single source for discovery, build-time translation, and runtime resolution.
pub const entries = [_]Entry{
    .{ .id = "costs", .path = "costs.fnl" },
    .{ .id = "history", .path = "history.fnl" },
    .{ .id = "queue", .path = "queue.fnl" },
    .{ .id = "queue_view", .path = "queue_view.fnl" },
    .{ .id = "images", .path = "images.fnl" },
    .{ .id = "attachments", .path = "attachments.fnl" },
    .{ .id = "component.image", .path = "component/image.fnl" },
    .{ .id = "actions", .path = "actions.fnl" },
    .{ .id = "clipboard", .path = "clipboard.fnl" },
    .{ .id = "selection_document", .path = "selection_document.fnl" },
    .{ .id = "selection", .path = "selection.fnl" },
    .{ .id = "component.selection", .path = "component/selection.fnl" },
    .{ .id = "editing", .path = "editing.fnl" },
    .{ .id = "agent", .path = "agent.fnl" },
    .{ .id = "auth", .path = "auth.fnl" },
    .{ .id = "choices", .path = "choices.fnl" },
    .{ .id = "choice_layout", .path = "choice_layout.fnl" },
    .{ .id = "choice_preview", .path = "choice_preview.fnl" },
    .{ .id = "values", .path = "values.fnl" },
    .{ .id = "usage", .path = "usage.fnl" },
    .{ .id = "links", .path = "links.fnl" },
    .{ .id = "component.buttons", .path = "component/buttons.fnl" },
    .{ .id = "component.data", .path = "component/data.fnl" },
    .{ .id = "commands", .path = "commands.fnl" },
    .{ .id = "omnipicker", .path = "omnipicker.fnl" },
    .{ .id = "dialogs", .path = "dialogs.fnl" },
    .{ .id = "dialog_view", .path = "dialog_view.fnl" },
    .{ .id = "components", .path = "components.fnl" },
    .{ .id = "layout", .path = "layout.fnl" },
    .{ .id = "markdown", .path = "markdown.fnl" },
    .{ .id = "syntax", .path = "syntax.fnl" },
    .{ .id = "component.markdown", .path = "component/markdown.fnl" },
    .{ .id = "component.tool", .path = "component/tool.fnl" },
    .{ .id = "component.message", .path = "component/message.fnl" },
    .{ .id = "component.editor", .path = "component/editor.fnl" },
    .{ .id = "component.picker", .path = "component/picker.fnl" },
    .{ .id = "component.status", .path = "component/status.fnl" },
    .{ .id = "component.chrome", .path = "component/chrome.fnl" },
    .{ .id = "component.dialog", .path = "component/dialog.fnl" },
    .{ .id = "editor", .path = "editor.fnl" },
    .{ .id = "fuzzy", .path = "fuzzy.fnl" },
    .{ .id = "keybindings", .path = "keybindings.fnl" },
    .{ .id = "indicators", .path = "indicators.fnl" },
    .{ .id = "json", .path = "json.fnl" },
    .{ .id = "messages", .path = "messages.fnl" },
    .{ .id = "models", .path = "models.fnl" },
    .{ .id = "picker", .path = "picker.fnl" },
    .{ .id = "picker_view", .path = "picker_view.fnl" },
    .{ .id = "preferences", .path = "preferences.fnl" },
    .{ .id = "request_options", .path = "request_options.fnl" },
    .{ .id = "effort", .path = "effort.fnl" },
    .{ .id = "status", .path = "status.fnl" },
    .{ .id = "themes", .path = "themes.fnl" },
    .{ .id = "theme.default", .path = "theme/default.fnl" },
    .{ .id = "animations", .path = "animations.fnl" },
    .{ .id = "animation.default", .path = "animation/default.fnl" },
    .{ .id = "provider.fake", .path = "provider/fake.fnl" },
    .{ .id = "provider.command", .path = "provider/command.fnl" },
    .{ .id = "provider.claude", .path = "provider/claude.fnl" },
    .{ .id = "protocol.anthropic", .path = "protocol/anthropic.fnl" },
    .{ .id = "provider.anthropic", .path = "provider/anthropic.fnl" },
    .{ .id = "provider.kimi", .path = "provider/kimi.fnl" },
    .{ .id = "protocol.openai", .path = "protocol/openai.fnl" },
    .{ .id = "provider.openai", .path = "provider/openai.fnl" },
    .{ .id = "provider.openai-codex", .path = "provider/openai-codex.fnl" },
    .{ .id = "provider.openrouter", .path = "provider/openrouter.fnl" },
    .{ .id = "tool.files", .path = "tool/files.fnl" },
    .{ .id = "tool.shell", .path = "tool/shell.fnl" },
    .{ .id = "ui", .path = "ui.fnl" },
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
    // luaL_loadfile accepts a C string, so reject truncation at this boundary
    // even when callers did not obtain the value through config.parse.
    if (std.mem.indexOfScalar(u8, value, 0) != null) return error.ExtensionPathContainsNul;
    if (isLiteralPath(value)) return allocator.dupe(u8, value);
    const relative = catalogPath(value) orelse return error.UnknownStandardExtension;
    const root = extension_dir orelse build_options.default_extension_dir;
    if (extension_dir != null) return std.fs.path.join(allocator, &.{ root, relative });
    const compiled = try std.fmt.allocPrint(allocator, "{s}.lua", .{relative[0 .. relative.len - 4]});
    defer allocator.free(compiled);
    return std.fs.path.join(allocator, &.{ root, compiled });
}

test "catalog entries are unique relative Fennel sources" {
    try std.testing.expect(entries.len > 0);
    for (entries, 0..) |entry, index| {
        try std.testing.expectEqualStrings(entry.id, ids[index]);
        try std.testing.expectEqualStrings(entry.path, catalogPath(entry.id).?);
        try std.testing.expect(entry.id.len > 0);
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

test "catalog accepts exact IDs only" {
    try std.testing.expectEqualStrings("agent.fnl", catalogPath("agent").?);
    try std.testing.expectEqualStrings("auth.fnl", catalogPath("auth").?);
    try std.testing.expectEqualStrings("choices.fnl", catalogPath("choices").?);
    try std.testing.expectEqualStrings("choice_layout.fnl", catalogPath("choice_layout").?);
    try std.testing.expectEqualStrings("commands.fnl", catalogPath("commands").?);
    try std.testing.expectEqualStrings("omnipicker.fnl", catalogPath("omnipicker").?);
    try std.testing.expectEqualStrings("dialogs.fnl", catalogPath("dialogs").?);
    try std.testing.expectEqualStrings("dialog_view.fnl", catalogPath("dialog_view").?);
    try std.testing.expectEqualStrings("components.fnl", catalogPath("components").?);
    try std.testing.expectEqualStrings("layout.fnl", catalogPath("layout").?);
    try std.testing.expectEqualStrings("markdown.fnl", catalogPath("markdown").?);
    try std.testing.expectEqualStrings("syntax.fnl", catalogPath("syntax").?);
    try std.testing.expectEqualStrings("component/markdown.fnl", catalogPath("component.markdown").?);
    try std.testing.expectEqualStrings("component/tool.fnl", catalogPath("component.tool").?);
    try std.testing.expectEqualStrings("component/message.fnl", catalogPath("component.message").?);
    try std.testing.expectEqualStrings("component/editor.fnl", catalogPath("component.editor").?);
    try std.testing.expectEqualStrings("component/picker.fnl", catalogPath("component.picker").?);
    try std.testing.expectEqualStrings("component/status.fnl", catalogPath("component.status").?);
    try std.testing.expectEqualStrings("component/chrome.fnl", catalogPath("component.chrome").?);
    try std.testing.expectEqualStrings("component/dialog.fnl", catalogPath("component.dialog").?);
    try std.testing.expect(catalogPath("component.default") == null);
    try std.testing.expectEqualStrings("editor.fnl", catalogPath("editor").?);
    try std.testing.expectEqualStrings("fuzzy.fnl", catalogPath("fuzzy").?);
    try std.testing.expectEqualStrings("keybindings.fnl", catalogPath("keybindings").?);
    try std.testing.expectEqualStrings("indicators.fnl", catalogPath("indicators").?);
    try std.testing.expectEqualStrings("json.fnl", catalogPath("json").?);
    try std.testing.expectEqualStrings("messages.fnl", catalogPath("messages").?);
    try std.testing.expectEqualStrings("models.fnl", catalogPath("models").?);
    try std.testing.expectEqualStrings("picker.fnl", catalogPath("picker").?);
    try std.testing.expectEqualStrings("picker_view.fnl", catalogPath("picker_view").?);
    try std.testing.expectEqualStrings("preferences.fnl", catalogPath("preferences").?);
    try std.testing.expectEqualStrings("request_options.fnl", catalogPath("request_options").?);
    try std.testing.expectEqualStrings("effort.fnl", catalogPath("effort").?);
    try std.testing.expectEqualStrings("status.fnl", catalogPath("status").?);
    try std.testing.expectEqualStrings("themes.fnl", catalogPath("themes").?);
    try std.testing.expectEqualStrings("theme/default.fnl", catalogPath("theme.default").?);
    try std.testing.expectEqualStrings("animations.fnl", catalogPath("animations").?);
    try std.testing.expectEqualStrings("animation/default.fnl", catalogPath("animation.default").?);
    try std.testing.expectEqualStrings("provider/fake.fnl", catalogPath("provider.fake").?);
    try std.testing.expectEqualStrings("provider/command.fnl", catalogPath("provider.command").?);
    try std.testing.expectEqualStrings("provider/claude.fnl", catalogPath("provider.claude").?);
    try std.testing.expectEqualStrings("protocol/anthropic.fnl", catalogPath("protocol.anthropic").?);
    try std.testing.expectEqualStrings("provider/anthropic.fnl", catalogPath("provider.anthropic").?);
    try std.testing.expectEqualStrings("provider/kimi.fnl", catalogPath("provider.kimi").?);
    try std.testing.expectEqualStrings("protocol/openai.fnl", catalogPath("protocol.openai").?);
    try std.testing.expectEqualStrings("provider/openai.fnl", catalogPath("provider.openai").?);
    try std.testing.expectEqualStrings("provider/openai-codex.fnl", catalogPath("provider.openai-codex").?);
    try std.testing.expectEqualStrings("provider/openrouter.fnl", catalogPath("provider.openrouter").?);
    try std.testing.expect(catalogPath("provider") == null);
    try std.testing.expect(catalogPath("Agent") == null);
    try std.testing.expectEqualStrings("ui.fnl", catalogPath("ui").?);
    try std.testing.expectEqualStrings("tool/files.fnl", catalogPath("tool.files").?);
    try std.testing.expectEqualStrings("tool/shell.fnl", catalogPath("tool.shell").?);
}

test "resolver preserves literals and resolves catalog roots" {
    const allocator = std.testing.allocator;
    const literal = try resolve(allocator, "custom/x.fnl", null);
    defer allocator.free(literal);
    try std.testing.expectEqualStrings("custom/x.fnl", literal);

    const env_path = try resolve(allocator, "provider.fake", "/source/extensions");
    defer allocator.free(env_path);
    try std.testing.expectEqualStrings("/source/extensions/provider/fake.fnl", env_path);

    const installed = try resolve(allocator, "agent", null);
    defer allocator.free(installed);
    try std.testing.expect(std.mem.endsWith(u8, installed, "/share/misa/extensions/agent.lua"));
    try std.testing.expectError(error.UnknownStandardExtension, resolve(allocator, "unknown", null));
    try std.testing.expectError(error.ExtensionPathContainsNul, resolve(allocator, "bad\x00.fnl", null));
}

test "resolver preserves legacy Lua extension paths" {
    const path = try resolve(std.testing.allocator, "custom.lua", null);
    defer std.testing.allocator.free(path);
    try std.testing.expectEqualStrings("custom.lua", path);
}
