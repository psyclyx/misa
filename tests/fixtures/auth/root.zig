//! Fixture authentication owns local storage and explicit CLI registrations.
//! There is no OAuth acquisition or refresh dependency in this composition.
const std = @import("std");
const auth = @import("misa_auth");

fn ownedPath(allocator: std.mem.Allocator, environ: *const std.process.Environ.Map, path: []const u8) !void {
    const root = environ.get("MISA_FIXTURE_ROOT") orelse return error.FixtureEnvironmentMissing;
    const resolved_root = try std.fs.path.resolve(allocator, &.{root});
    defer allocator.free(resolved_root);
    const resolved = try std.fs.path.resolve(allocator, &.{path});
    defer allocator.free(resolved);
    if (!std.mem.startsWith(u8, resolved, resolved_root) or resolved.len <= resolved_root.len or resolved[resolved_root.len] != std.fs.path.sep)
        return error.FixturePathOutsideRoot;
}

fn localCommand(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: auth.Action, provider: []const u8, account: ?[]const u8, interaction: ?auth.Interaction) !auth.CommandResult {
    if (account) |name| if (!auth.validAccountName(name)) return error.InvalidAccount;
    if (std.mem.eql(u8, provider, "claude")) {
        // Claude Code owns its single account: Misa stores none for it.
        if (account != null or action == .select) return error.UnknownAccount;
        const executable = environ.get("MISA_FIXTURE_CLAUDE") orelse return error.UnmatchedAuthFixture;
        try ownedPath(allocator, environ, executable);
        return auth.commandCli(allocator, io, environ, action, executable);
    }
    const api_key = std.mem.eql(u8, provider, "openai") or std.mem.eql(u8, provider, "anthropic");
    const oauth = std.mem.eql(u8, provider, "openai-codex") or std.mem.eql(u8, provider, "kimi-coding") or std.mem.eql(u8, provider, "openrouter");
    if (!api_key and !oauth) return error.UnknownProvider;
    if (action == .login and !api_key) return error.UnmatchedAuthFixture;
    const path = try auth.credentialPath(allocator, environ);
    defer allocator.free(path);
    try ownedPath(allocator, environ, path);
    var store = try auth.Store.init(allocator, io, environ);
    defer store.deinit();
    switch (action) {
        .status => return auth.statusResult(allocator, &store, provider, account),
        .select => return auth.selectResult(allocator, &store, provider, account orelse return error.MissingAccount),
        .logout => {
            if (account) |name| {
                if (!try store.removeAccount(provider, name)) return error.UnknownAccount;
                return .{ .logged_in = false, .account = try allocator.dupe(u8, name), .message = try auth.logoutMessage(allocator, name) };
            }
            _ = try store.remove(provider);
            return .{ .logged_in = false, .message = try auth.logoutMessage(allocator, null) };
        },
        .login => {},
    }
    // Without a named account, login replaces the credential in use.
    const target = account orelse store.activeAccount(provider) orelse auth.default_account;
    if (interaction) |channel| {
        if (channel.protected_input) {
            try channel.emit(.{ .correlation = "api-key", .kind = "modal", .title = "API key", .message = "Enter your API key. Characters are hidden.", .input = true, .protected = true });
            const secret = try channel.input("api-key");
            if (secret.len == 0) return error.EmptyCredential;
            try store.putAccount(provider, target, secret);
            return .{ .logged_in = true, .account = try allocator.dupe(u8, target), .message = try auth.loginMessage(allocator, target) };
        }
    }
    const secret = try auth.readSecret(allocator, io, "API key: ");
    defer {
        std.crypto.secureZero(u8, secret);
        allocator.free(secret);
    }
    try store.putAccount(provider, target, secret);
    if (std.mem.eql(u8, target, auth.default_account))
        std.debug.print("misa: saved {s} credential to {s}\n", .{ provider, store.path })
    else
        std.debug.print("misa: saved {s} credential for account {s} to {s}\n", .{ provider, target, store.path });
    return .{ .logged_in = true, .account = try allocator.dupe(u8, target), .message = try auth.loginMessage(allocator, target) };
}

pub fn commandTerminal(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: auth.Action, provider: []const u8, account: ?[]const u8) !auth.CommandResult {
    return localCommand(allocator, io, environ, action, provider, account, null);
}

pub fn command(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: auth.Action, declaration: auth.Declaration, interaction: auth.Interaction, account: ?[]const u8) !auth.CommandResult {
    try declaration.validate();
    return localCommand(allocator, io, environ, action, declaration.provider, account, interaction);
}
