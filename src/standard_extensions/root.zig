//! Catalog and path resolver for extensions shipped with misa.
const std = @import("std");
const build_options = @import("misa_build_options");

pub const default_config_path = build_options.default_config_path;

pub const ids = [_][]const u8{
    "agent",
    "auth",
    "choices",
    "dialogs",
    "dialog_view",
    "components",
    "layout",
    "markdown",
    "component.markdown",
    "component.message",
    "component.editor",
    "component.picker",
    "component.status",
    "component.chrome",
    "component.dialog",
    "editor",
    "fuzzy",
    "keybindings",
    "indicators",
    "json",
    "messages",
    "models",
    "picker",
    "picker_view",
    "preferences",
    "request_options",
    "effort",
    "status",
    "themes",
    "theme.default",
    "animations",
    "animation.default",
    "provider.fake",
    "provider.command",
    "provider.claude",
    "protocol.anthropic",
    "provider.anthropic",
    "provider.kimi",
    "protocol.openai",
    "provider.openai",
    "provider.openai-codex",
    "provider.openrouter",
    "tool.files",
    "tool.shell",
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
    if (std.mem.eql(u8, id, "auth")) return "auth.lua";
    if (std.mem.eql(u8, id, "choices")) return "choices.lua";
    if (std.mem.eql(u8, id, "dialogs")) return "dialogs.lua";
    if (std.mem.eql(u8, id, "dialog_view")) return "dialog_view.lua";
    if (std.mem.eql(u8, id, "components")) return "components.lua";
    if (std.mem.eql(u8, id, "layout")) return "layout.lua";
    if (std.mem.eql(u8, id, "markdown")) return "markdown.lua";
    if (std.mem.eql(u8, id, "component.markdown")) return "component/markdown.lua";
    if (std.mem.eql(u8, id, "component.message")) return "component/message.lua";
    if (std.mem.eql(u8, id, "component.editor")) return "component/editor.lua";
    if (std.mem.eql(u8, id, "component.picker")) return "component/picker.lua";
    if (std.mem.eql(u8, id, "component.status")) return "component/status.lua";
    if (std.mem.eql(u8, id, "component.chrome")) return "component/chrome.lua";
    if (std.mem.eql(u8, id, "component.dialog")) return "component/dialog.lua";
    if (std.mem.eql(u8, id, "editor")) return "editor.lua";
    if (std.mem.eql(u8, id, "fuzzy")) return "fuzzy.lua";
    if (std.mem.eql(u8, id, "keybindings")) return "keybindings.lua";
    if (std.mem.eql(u8, id, "indicators")) return "indicators.lua";
    if (std.mem.eql(u8, id, "json")) return "json.lua";
    if (std.mem.eql(u8, id, "messages")) return "messages.lua";
    if (std.mem.eql(u8, id, "models")) return "models.lua";
    if (std.mem.eql(u8, id, "picker")) return "picker.lua";
    if (std.mem.eql(u8, id, "picker_view")) return "picker_view.lua";
    if (std.mem.eql(u8, id, "preferences")) return "preferences.lua";
    if (std.mem.eql(u8, id, "request_options")) return "request_options.lua";
    if (std.mem.eql(u8, id, "effort")) return "effort.lua";
    if (std.mem.eql(u8, id, "status")) return "status.lua";
    if (std.mem.eql(u8, id, "themes")) return "themes.lua";
    if (std.mem.eql(u8, id, "theme.default")) return "theme/default.lua";
    if (std.mem.eql(u8, id, "animations")) return "animations.lua";
    if (std.mem.eql(u8, id, "animation.default")) return "animation/default.lua";
    if (std.mem.eql(u8, id, "provider.fake")) return "provider/fake.lua";
    if (std.mem.eql(u8, id, "provider.command")) return "provider/command.lua";
    if (std.mem.eql(u8, id, "provider.claude")) return "provider/claude.lua";
    if (std.mem.eql(u8, id, "protocol.anthropic")) return "protocol/anthropic.lua";
    if (std.mem.eql(u8, id, "provider.anthropic")) return "provider/anthropic.lua";
    if (std.mem.eql(u8, id, "provider.kimi")) return "provider/kimi.lua";
    if (std.mem.eql(u8, id, "protocol.openai")) return "protocol/openai.lua";
    if (std.mem.eql(u8, id, "provider.openai")) return "provider/openai.lua";
    if (std.mem.eql(u8, id, "provider.openai-codex")) return "provider/openai-codex.lua";
    if (std.mem.eql(u8, id, "provider.openrouter")) return "provider/openrouter.lua";
    if (std.mem.eql(u8, id, "tool.files")) return "tool/files.lua";
    if (std.mem.eql(u8, id, "tool.shell")) return "tool/shell.lua";
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
    try std.testing.expectEqualStrings("auth.lua", catalogPath("auth").?);
    try std.testing.expectEqualStrings("choices.lua", catalogPath("choices").?);
    try std.testing.expectEqualStrings("dialogs.lua", catalogPath("dialogs").?);
    try std.testing.expectEqualStrings("dialog_view.lua", catalogPath("dialog_view").?);
    try std.testing.expectEqualStrings("components.lua", catalogPath("components").?);
    try std.testing.expectEqualStrings("layout.lua", catalogPath("layout").?);
    try std.testing.expectEqualStrings("markdown.lua", catalogPath("markdown").?);
    try std.testing.expectEqualStrings("component/markdown.lua", catalogPath("component.markdown").?);
    try std.testing.expectEqualStrings("component/message.lua", catalogPath("component.message").?);
    try std.testing.expectEqualStrings("component/editor.lua", catalogPath("component.editor").?);
    try std.testing.expectEqualStrings("component/picker.lua", catalogPath("component.picker").?);
    try std.testing.expectEqualStrings("component/status.lua", catalogPath("component.status").?);
    try std.testing.expectEqualStrings("component/chrome.lua", catalogPath("component.chrome").?);
    try std.testing.expectEqualStrings("component/dialog.lua", catalogPath("component.dialog").?);
    try std.testing.expect(catalogPath("component.default") == null);
    try std.testing.expectEqualStrings("editor.lua", catalogPath("editor").?);
    try std.testing.expectEqualStrings("fuzzy.lua", catalogPath("fuzzy").?);
    try std.testing.expectEqualStrings("keybindings.lua", catalogPath("keybindings").?);
    try std.testing.expectEqualStrings("indicators.lua", catalogPath("indicators").?);
    try std.testing.expectEqualStrings("json.lua", catalogPath("json").?);
    try std.testing.expectEqualStrings("messages.lua", catalogPath("messages").?);
    try std.testing.expectEqualStrings("models.lua", catalogPath("models").?);
    try std.testing.expectEqualStrings("picker.lua", catalogPath("picker").?);
    try std.testing.expectEqualStrings("picker_view.lua", catalogPath("picker_view").?);
    try std.testing.expectEqualStrings("preferences.lua", catalogPath("preferences").?);
    try std.testing.expectEqualStrings("request_options.lua", catalogPath("request_options").?);
    try std.testing.expectEqualStrings("effort.lua", catalogPath("effort").?);
    try std.testing.expectEqualStrings("status.lua", catalogPath("status").?);
    try std.testing.expectEqualStrings("themes.lua", catalogPath("themes").?);
    try std.testing.expectEqualStrings("theme/default.lua", catalogPath("theme.default").?);
    try std.testing.expectEqualStrings("animations.lua", catalogPath("animations").?);
    try std.testing.expectEqualStrings("animation/default.lua", catalogPath("animation.default").?);
    try std.testing.expectEqualStrings("provider/fake.lua", catalogPath("provider.fake").?);
    try std.testing.expectEqualStrings("provider/command.lua", catalogPath("provider.command").?);
    try std.testing.expectEqualStrings("provider/claude.lua", catalogPath("provider.claude").?);
    try std.testing.expectEqualStrings("protocol/anthropic.lua", catalogPath("protocol.anthropic").?);
    try std.testing.expectEqualStrings("provider/anthropic.lua", catalogPath("provider.anthropic").?);
    try std.testing.expectEqualStrings("provider/kimi.lua", catalogPath("provider.kimi").?);
    try std.testing.expectEqualStrings("protocol/openai.lua", catalogPath("protocol.openai").?);
    try std.testing.expectEqualStrings("provider/openai.lua", catalogPath("provider.openai").?);
    try std.testing.expectEqualStrings("provider/openai-codex.lua", catalogPath("provider.openai-codex").?);
    try std.testing.expectEqualStrings("provider/openrouter.lua", catalogPath("provider.openrouter").?);
    try std.testing.expect(catalogPath("provider") == null);
    try std.testing.expect(catalogPath("Agent") == null);
    try std.testing.expectEqualStrings("ui.lua", catalogPath("ui").?);
    try std.testing.expectEqualStrings("tool/files.lua", catalogPath("tool.files").?);
    try std.testing.expectEqualStrings("tool/shell.lua", catalogPath("tool.shell").?);
    try std.testing.expectEqual(@as(usize, 45), ids.len);
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
