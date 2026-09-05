//! Catalog and path resolver for extensions shipped with misa.
const std = @import("std");
const build_options = @import("misa_build_options");

pub const default_config_path = build_options.default_config_path;

pub const ids = [_][]const u8{
    "costs",
    "history",
    "queue",
    "queue_view",
    "images",
    "attachments",
    "component.image",

    "actions",
    "clipboard",
    "selection_document",
    "selection",
    "component.selection",
    "editing",
    "agent",
    "auth",
    "choices",

    "choice_layout",
    "commands",
    "omnipicker",
    "dialogs",
    "dialog_view",
    "components",
    "layout",
    "markdown",
    "component.markdown",
    "component.tool",
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
    return std.mem.indexOfScalar(u8, value, '/') != null or std.mem.endsWith(u8, value, ".fnl") or std.mem.endsWith(u8, value, ".lua");
}

pub fn catalogPath(id: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, id, "actions")) return "actions.fnl";
    if (std.mem.eql(u8, id, "clipboard")) return "clipboard.fnl";
    if (std.mem.eql(u8, id, "selection_document")) return "selection_document.fnl";
    if (std.mem.eql(u8, id, "selection")) return "selection.fnl";
    if (std.mem.eql(u8, id, "component.selection")) return "component/selection.fnl";
    if (std.mem.eql(u8, id, "editing")) return "editing.fnl";
    if (std.mem.eql(u8, id, "costs")) return "costs.fnl";
    if (std.mem.eql(u8, id, "history")) return "history.fnl";
    if (std.mem.eql(u8, id, "queue")) return "queue.fnl";
    if (std.mem.eql(u8, id, "queue_view")) return "queue_view.fnl";
    if (std.mem.eql(u8, id, "images")) return "images.fnl";
    if (std.mem.eql(u8, id, "attachments")) return "attachments.fnl";
    if (std.mem.eql(u8, id, "component.image")) return "component/image.fnl";

    if (std.mem.eql(u8, id, "agent")) return "agent.fnl";
    if (std.mem.eql(u8, id, "auth")) return "auth.fnl";
    if (std.mem.eql(u8, id, "choices")) return "choices.fnl";
    if (std.mem.eql(u8, id, "choice_layout")) return "choice_layout.fnl";
    if (std.mem.eql(u8, id, "commands")) return "commands.fnl";
    if (std.mem.eql(u8, id, "omnipicker")) return "omnipicker.fnl";
    if (std.mem.eql(u8, id, "dialogs")) return "dialogs.fnl";
    if (std.mem.eql(u8, id, "dialog_view")) return "dialog_view.fnl";
    if (std.mem.eql(u8, id, "components")) return "components.fnl";
    if (std.mem.eql(u8, id, "layout")) return "layout.fnl";
    if (std.mem.eql(u8, id, "markdown")) return "markdown.fnl";
    if (std.mem.eql(u8, id, "component.markdown")) return "component/markdown.fnl";
    if (std.mem.eql(u8, id, "component.tool")) return "component/tool.fnl";
    if (std.mem.eql(u8, id, "component.message")) return "component/message.fnl";
    if (std.mem.eql(u8, id, "component.editor")) return "component/editor.fnl";
    if (std.mem.eql(u8, id, "component.picker")) return "component/picker.fnl";
    if (std.mem.eql(u8, id, "component.status")) return "component/status.fnl";
    if (std.mem.eql(u8, id, "component.chrome")) return "component/chrome.fnl";
    if (std.mem.eql(u8, id, "component.dialog")) return "component/dialog.fnl";
    if (std.mem.eql(u8, id, "editor")) return "editor.fnl";
    if (std.mem.eql(u8, id, "fuzzy")) return "fuzzy.fnl";
    if (std.mem.eql(u8, id, "keybindings")) return "keybindings.fnl";
    if (std.mem.eql(u8, id, "indicators")) return "indicators.fnl";
    if (std.mem.eql(u8, id, "json")) return "json.fnl";
    if (std.mem.eql(u8, id, "messages")) return "messages.fnl";
    if (std.mem.eql(u8, id, "models")) return "models.fnl";
    if (std.mem.eql(u8, id, "picker")) return "picker.fnl";
    if (std.mem.eql(u8, id, "picker_view")) return "picker_view.fnl";
    if (std.mem.eql(u8, id, "preferences")) return "preferences.fnl";
    if (std.mem.eql(u8, id, "request_options")) return "request_options.fnl";
    if (std.mem.eql(u8, id, "effort")) return "effort.fnl";
    if (std.mem.eql(u8, id, "status")) return "status.fnl";
    if (std.mem.eql(u8, id, "themes")) return "themes.fnl";
    if (std.mem.eql(u8, id, "theme.default")) return "theme/default.fnl";
    if (std.mem.eql(u8, id, "animations")) return "animations.fnl";
    if (std.mem.eql(u8, id, "animation.default")) return "animation/default.fnl";
    if (std.mem.eql(u8, id, "provider.fake")) return "provider/fake.fnl";
    if (std.mem.eql(u8, id, "provider.command")) return "provider/command.fnl";
    if (std.mem.eql(u8, id, "provider.claude")) return "provider/claude.fnl";
    if (std.mem.eql(u8, id, "protocol.anthropic")) return "protocol/anthropic.fnl";
    if (std.mem.eql(u8, id, "provider.anthropic")) return "provider/anthropic.fnl";
    if (std.mem.eql(u8, id, "provider.kimi")) return "provider/kimi.fnl";
    if (std.mem.eql(u8, id, "protocol.openai")) return "protocol/openai.fnl";
    if (std.mem.eql(u8, id, "provider.openai")) return "provider/openai.fnl";
    if (std.mem.eql(u8, id, "provider.openai-codex")) return "provider/openai-codex.fnl";
    if (std.mem.eql(u8, id, "provider.openrouter")) return "provider/openrouter.fnl";
    if (std.mem.eql(u8, id, "tool.files")) return "tool/files.fnl";
    if (std.mem.eql(u8, id, "tool.shell")) return "tool/shell.fnl";
    if (std.mem.eql(u8, id, "ui")) return "ui.fnl";
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
    try std.testing.expectEqual(@as(usize, 62), ids.len);
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
    try std.testing.expect(std.mem.endsWith(u8, installed, "/share/misa/extensions/agent.fnl"));
    try std.testing.expectError(error.UnknownStandardExtension, resolve(allocator, "unknown", null));
    try std.testing.expectError(error.ExtensionPathContainsNul, resolve(allocator, "bad\x00.fnl", null));
}

test "resolver preserves legacy Lua extension paths" {
    const path = try resolve(std.testing.allocator, "custom.lua", null);
    defer std.testing.allocator.free(path);
    try std.testing.expectEqualStrings("custom.lua", path);
}
