//! Catalog and path resolver for extensions shipped with misa.
const std = @import("std");
const build_options = @import("misa_build_options");

pub const default_config_path = build_options.default_config_path;
pub const default_extension_dir = build_options.default_extension_dir;

pub const Entry = struct { id: []const u8, path: []const u8 };

/// Single source for discovery, build-time translation, and runtime resolution.
pub const entries = [_]Entry{
    .{ .id = "misa.actions", .path = "misa/actions.fnl" },
    .{ .id = "misa.agent", .path = "misa/agent/init.fnl" },
    .{ .id = "misa.agent.stream", .path = "misa/agent/stream.fnl" },
    .{ .id = "misa.choices", .path = "misa/choices/init.fnl" },
    .{ .id = "misa.choices.layout", .path = "misa/choices/layout.fnl" },
    .{ .id = "misa.choices.matching", .path = "misa/choices/matching.fnl" },
    .{ .id = "misa.choices.picker", .path = "misa/choices/picker/init.fnl" },
    .{ .id = "misa.choices.picker.render", .path = "misa/choices/picker/render.fnl" },
    .{ .id = "misa.choices.picker.view", .path = "misa/choices/picker/view.fnl" },
    .{ .id = "misa.choices.preferences", .path = "misa/choices/preferences.fnl" },
    .{ .id = "misa.choices.preview", .path = "misa/choices/preview.fnl" },
    .{ .id = "misa.clipboard", .path = "misa/clipboard.fnl" },
    .{ .id = "misa.commands", .path = "misa/commands/init.fnl" },
    .{ .id = "misa.commands.palette", .path = "misa/commands/palette.fnl" },
    .{ .id = "misa.costs", .path = "misa/costs.fnl" },
    .{ .id = "misa.dialogs", .path = "misa/dialogs/init.fnl" },
    .{ .id = "misa.dialogs.render", .path = "misa/dialogs/render.fnl" },
    .{ .id = "misa.dialogs.view", .path = "misa/dialogs/view.fnl" },
    .{ .id = "misa.editor", .path = "misa/editor/init.fnl" },
    .{ .id = "misa.editor.attachments", .path = "misa/editor/attachments.fnl" },
    .{ .id = "misa.editor.editing", .path = "misa/editor/editing.fnl" },
    .{ .id = "misa.editor.history", .path = "misa/editor/history.fnl" },
    .{ .id = "misa.editor.images", .path = "misa/editor/images/init.fnl" },
    .{ .id = "misa.editor.images.render", .path = "misa/editor/images/render.fnl" },
    .{ .id = "misa.editor.queue", .path = "misa/editor/queue/init.fnl" },
    .{ .id = "misa.editor.queue.view", .path = "misa/editor/queue/view.fnl" },
    .{ .id = "misa.editor.render", .path = "misa/editor/render.fnl" },
    .{ .id = "misa.json", .path = "misa/json.fnl" },
    .{ .id = "misa.keybindings", .path = "misa/keybindings.fnl" },
    .{ .id = "misa.links", .path = "misa/links.fnl" },
    .{ .id = "misa.markdown", .path = "misa/markdown/init.fnl" },
    .{ .id = "misa.markdown.render", .path = "misa/markdown/render.fnl" },
    .{ .id = "misa.models", .path = "misa/models/init.fnl" },
    .{ .id = "misa.models.effort", .path = "misa/models/effort.fnl" },
    .{ .id = "misa.models.options", .path = "misa/models/options.fnl" },
    .{ .id = "misa.models.preview", .path = "misa/models/preview.fnl" },
    .{ .id = "misa.protocols.anthropic", .path = "misa/protocols/anthropic.fnl" },
    .{ .id = "misa.protocols.openai", .path = "misa/protocols/openai.fnl" },
    .{ .id = "misa.providers.anthropic", .path = "misa/providers/anthropic.fnl" },
    .{ .id = "misa.providers.auth", .path = "misa/providers/auth.fnl" },
    .{ .id = "misa.providers.claude", .path = "misa/providers/claude.fnl" },
    .{ .id = "misa.providers.command", .path = "misa/providers/command.fnl" },
    .{ .id = "misa.providers.fake", .path = "misa/providers/fake.fnl" },
    .{ .id = "misa.providers.kimi", .path = "misa/providers/kimi.fnl" },
    .{ .id = "misa.providers.openai", .path = "misa/providers/openai.fnl" },
    .{ .id = "misa.providers.openai-codex", .path = "misa/providers/openai-codex.fnl" },
    .{ .id = "misa.providers.openai-options", .path = "misa/providers/openai-options.fnl" },
    .{ .id = "misa.providers.openrouter", .path = "misa/providers/openrouter.fnl" },
    .{ .id = "misa.selection", .path = "misa/selection/init.fnl" },
    .{ .id = "misa.selection.document", .path = "misa/selection/document.fnl" },
    .{ .id = "misa.selection.render", .path = "misa/selection/render.fnl" },
    .{ .id = "misa.standard", .path = "misa/standard/init.fnl" },
    .{ .id = "misa.standard.actions", .path = "misa/standard/actions.fnl" },
    .{ .id = "misa.standard.agent", .path = "misa/standard/agent/init.fnl" },
    .{ .id = "misa.standard.agent.stream", .path = "misa/standard/agent/stream.fnl" },
    .{ .id = "misa.standard.choices", .path = "misa/standard/choices/init.fnl" },
    .{ .id = "misa.standard.choices.layout", .path = "misa/standard/choices/layout.fnl" },
    .{ .id = "misa.standard.choices.matching", .path = "misa/standard/choices/matching.fnl" },
    .{ .id = "misa.standard.choices.picker", .path = "misa/standard/choices/picker/init.fnl" },
    .{ .id = "misa.standard.choices.picker.render", .path = "misa/standard/choices/picker/render.fnl" },
    .{ .id = "misa.standard.choices.picker.view", .path = "misa/standard/choices/picker/view.fnl" },
    .{ .id = "misa.standard.choices.preferences", .path = "misa/standard/choices/preferences.fnl" },
    .{ .id = "misa.standard.choices.preview", .path = "misa/standard/choices/preview.fnl" },
    .{ .id = "misa.standard.clipboard", .path = "misa/standard/clipboard.fnl" },
    .{ .id = "misa.standard.commands", .path = "misa/standard/commands/init.fnl" },
    .{ .id = "misa.standard.commands.palette", .path = "misa/standard/commands/palette.fnl" },
    .{ .id = "misa.standard.costs", .path = "misa/standard/costs.fnl" },
    .{ .id = "misa.standard.dialogs", .path = "misa/standard/dialogs/init.fnl" },
    .{ .id = "misa.standard.dialogs.render", .path = "misa/standard/dialogs/render.fnl" },
    .{ .id = "misa.standard.dialogs.view", .path = "misa/standard/dialogs/view.fnl" },
    .{ .id = "misa.standard.editor", .path = "misa/standard/editor/init.fnl" },
    .{ .id = "misa.standard.editor.attachments", .path = "misa/standard/editor/attachments.fnl" },
    .{ .id = "misa.standard.editor.editing", .path = "misa/standard/editor/editing.fnl" },
    .{ .id = "misa.standard.editor.history", .path = "misa/standard/editor/history.fnl" },
    .{ .id = "misa.standard.editor.images", .path = "misa/standard/editor/images/init.fnl" },
    .{ .id = "misa.standard.editor.images.render", .path = "misa/standard/editor/images/render.fnl" },
    .{ .id = "misa.standard.editor.queue", .path = "misa/standard/editor/queue/init.fnl" },
    .{ .id = "misa.standard.editor.queue.view", .path = "misa/standard/editor/queue/view.fnl" },
    .{ .id = "misa.standard.editor.render", .path = "misa/standard/editor/render.fnl" },
    .{ .id = "misa.standard.json", .path = "misa/standard/json.fnl" },
    .{ .id = "misa.standard.keybindings", .path = "misa/standard/keybindings.fnl" },
    .{ .id = "misa.standard.links", .path = "misa/standard/links.fnl" },
    .{ .id = "misa.standard.models", .path = "misa/standard/models/init.fnl" },
    .{ .id = "misa.standard.models.effort", .path = "misa/standard/models/effort.fnl" },
    .{ .id = "misa.standard.models.options", .path = "misa/standard/models/options.fnl" },
    .{ .id = "misa.standard.models.preview", .path = "misa/standard/models/preview.fnl" },
    .{ .id = "misa.standard.presentation", .path = "misa/standard/presentation/init.fnl" },
    .{ .id = "misa.standard.presentation.animations", .path = "misa/standard/presentation/animations.fnl" },
    .{ .id = "misa.standard.presentation.components", .path = "misa/standard/presentation/components.fnl" },
    .{ .id = "misa.standard.presentation.elements", .path = "misa/standard/presentation/elements.fnl" },
    .{ .id = "misa.standard.presentation.markdown", .path = "misa/standard/presentation/markdown.fnl" },
    .{ .id = "misa.standard.presentation.status", .path = "misa/standard/presentation/status.fnl" },
    .{ .id = "misa.standard.presentation.syntax", .path = "misa/standard/presentation/syntax.fnl" },
    .{ .id = "misa.standard.presentation.theme", .path = "misa/standard/presentation/theme.fnl" },
    .{ .id = "misa.standard.presentation.tools", .path = "misa/standard/presentation/tools.fnl" },
    .{ .id = "misa.standard.presentation.transcript", .path = "misa/standard/presentation/transcript.fnl" },
    .{ .id = "misa.standard.presentation.ui", .path = "misa/standard/presentation/ui.fnl" },
    .{ .id = "misa.standard.presentation.values", .path = "misa/standard/presentation/values.fnl" },
    .{ .id = "misa.standard.protocols.anthropic", .path = "misa/standard/protocols/anthropic.fnl" },
    .{ .id = "misa.standard.protocols.openai", .path = "misa/standard/protocols/openai.fnl" },
    .{ .id = "misa.standard.providers", .path = "misa/standard/providers/init.fnl" },
    .{ .id = "misa.standard.providers.anthropic", .path = "misa/standard/providers/anthropic.fnl" },
    .{ .id = "misa.standard.providers.auth", .path = "misa/standard/providers/auth.fnl" },
    .{ .id = "misa.standard.providers.cerebras", .path = "misa/standard/providers/cerebras.fnl" },
    .{ .id = "misa.standard.providers.claude", .path = "misa/standard/providers/claude.fnl" },
    .{ .id = "misa.standard.providers.command", .path = "misa/standard/providers/command.fnl" },
    .{ .id = "misa.standard.providers.deepinfra", .path = "misa/standard/providers/deepinfra.fnl" },
    .{ .id = "misa.standard.providers.deepseek", .path = "misa/standard/providers/deepseek.fnl" },
    .{ .id = "misa.standard.providers.fake", .path = "misa/standard/providers/fake.fnl" },
    .{ .id = "misa.standard.providers.fireworks", .path = "misa/standard/providers/fireworks.fnl" },
    .{ .id = "misa.standard.providers.generic", .path = "misa/standard/providers/generic.fnl" },
    .{ .id = "misa.standard.providers.groq", .path = "misa/standard/providers/groq.fnl" },
    .{ .id = "misa.standard.providers.huggingface", .path = "misa/standard/providers/huggingface.fnl" },
    .{ .id = "misa.standard.providers.kimi", .path = "misa/standard/providers/kimi.fnl" },
    .{ .id = "misa.standard.providers.mistral", .path = "misa/standard/providers/mistral.fnl" },
    .{ .id = "misa.standard.providers.moonshot", .path = "misa/standard/providers/moonshot.fnl" },
    .{ .id = "misa.standard.providers.novita", .path = "misa/standard/providers/novita.fnl" },
    .{ .id = "misa.standard.providers.nvidia", .path = "misa/standard/providers/nvidia.fnl" },
    .{ .id = "misa.standard.providers.openai", .path = "misa/standard/providers/openai.fnl" },
    .{ .id = "misa.standard.providers.openai-codex", .path = "misa/standard/providers/openai-codex.fnl" },
    .{ .id = "misa.standard.providers.openai-compatible", .path = "misa/standard/providers/openai-compatible.fnl" },
    .{ .id = "misa.standard.providers.openrouter", .path = "misa/standard/providers/openrouter.fnl" },
    .{ .id = "misa.standard.providers.siliconflow", .path = "misa/standard/providers/siliconflow.fnl" },
    .{ .id = "misa.standard.providers.together", .path = "misa/standard/providers/together.fnl" },
    .{ .id = "misa.standard.providers.venice", .path = "misa/standard/providers/venice.fnl" },
    .{ .id = "misa.standard.providers.xai", .path = "misa/standard/providers/xai.fnl" },
    .{ .id = "misa.standard.selection", .path = "misa/standard/selection/init.fnl" },
    .{ .id = "misa.standard.selection.document", .path = "misa/standard/selection/document.fnl" },
    .{ .id = "misa.standard.selection.render", .path = "misa/standard/selection/render.fnl" },
    .{ .id = "misa.standard.settings", .path = "misa/standard/settings.fnl" },
    .{ .id = "misa.standard.tools.files", .path = "misa/standard/tools/files.fnl" },
    .{ .id = "misa.standard.tools.shell", .path = "misa/standard/tools/shell.fnl" },
    .{ .id = "misa.standard.usage", .path = "misa/standard/usage/init.fnl" },
    .{ .id = "misa.standard.usage.dialog", .path = "misa/standard/usage/dialog.fnl" },
    .{ .id = "misa.tools.files", .path = "misa/tools/files.fnl" },
    .{ .id = "misa.tools.shell", .path = "misa/tools/shell.fnl" },
    .{ .id = "misa.transcript", .path = "misa/transcript/init.fnl" },
    .{ .id = "misa.transcript.groups", .path = "misa/transcript/groups.fnl" },
    .{ .id = "misa.transcript.model", .path = "misa/transcript/model.fnl" },
    .{ .id = "misa.transcript.presentation", .path = "misa/transcript/presentation.fnl" },
    .{ .id = "misa.transcript.render", .path = "misa/transcript/render.fnl" },
    .{ .id = "misa.transcript.syntax", .path = "misa/transcript/syntax.fnl" },
    .{ .id = "misa.transcript.tools", .path = "misa/transcript/tools/init.fnl" },
    .{ .id = "misa.transcript.tools.render", .path = "misa/transcript/tools/render.fnl" },
    .{ .id = "misa.transcript.tools.summary", .path = "misa/transcript/tools/summary.fnl" },
    .{ .id = "misa.transcript.viewport", .path = "misa/transcript/viewport.fnl" },
    .{ .id = "misa.ui", .path = "misa/ui/init.fnl" },
    .{ .id = "misa.ui.animations", .path = "misa/ui/animations/init.fnl" },
    .{ .id = "misa.ui.animations.default", .path = "misa/ui/animations/default.fnl" },
    .{ .id = "misa.ui.animations.state", .path = "misa/ui/animations/state.fnl" },
    .{ .id = "misa.ui.chrome", .path = "misa/ui/chrome.fnl" },
    .{ .id = "misa.ui.components", .path = "misa/ui/components/init.fnl" },
    .{ .id = "misa.ui.components.buttons", .path = "misa/ui/components/buttons.fnl" },
    .{ .id = "misa.ui.components.content", .path = "misa/ui/components/content.fnl" },
    .{ .id = "misa.ui.components.data", .path = "misa/ui/components/data.fnl" },
    .{ .id = "misa.ui.components.group", .path = "misa/ui/components/group.fnl" },
    .{ .id = "misa.ui.components.truncation", .path = "misa/ui/components/truncation.fnl" },
    .{ .id = "misa.ui.components.validation", .path = "misa/ui/components/validation.fnl" },
    .{ .id = "misa.ui.layout", .path = "misa/ui/layout.fnl" },
    .{ .id = "misa.ui.status", .path = "misa/ui/status/init.fnl" },
    .{ .id = "misa.ui.status.indicators", .path = "misa/ui/status/indicators.fnl" },
    .{ .id = "misa.ui.status.render", .path = "misa/ui/status/render.fnl" },
    .{ .id = "misa.ui.themes", .path = "misa/ui/themes/init.fnl" },
    .{ .id = "misa.ui.themes.default", .path = "misa/ui/themes/default.fnl" },
    .{ .id = "misa.ui.themes.styles", .path = "misa/ui/themes/styles.fnl" },
    .{ .id = "misa.ui.values", .path = "misa/ui/values.fnl" },
    .{ .id = "misa.usage", .path = "misa/usage/init.fnl" },
    .{ .id = "misa.usage.dialog", .path = "misa/usage/dialog.fnl" },
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
    try std.testing.expectEqualStrings("misa/markdown/render.fnl", catalogPath("misa.markdown.render").?);
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
