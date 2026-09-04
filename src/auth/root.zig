//! XDG credential storage and explicit login commands.
const std = @import("std");
const posix = std.posix;
pub const oauth = @import("oauth.zig");

pub const Store = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    path: []u8,
    parsed: std.json.Parsed(std.json.Value),
    protect_parent: bool,

    pub fn init(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) !Store {
        const path = try credentialPath(allocator, environ);
        errdefer allocator.free(path);
        const source = std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024)) catch |err| switch (err) {
            error.FileNotFound => try allocator.dupe(u8, "{}"),
            else => return err,
        };
        defer allocator.free(source);
        var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source, .{ .allocate = .alloc_always });
        errdefer parsed.deinit();
        if (parsed.value != .object) return error.InvalidCredentialStore;
        return .{
            .allocator = allocator,
            .io = io,
            .path = path,
            .parsed = parsed,
            .protect_parent = environ.get("MISA_AUTH_FILE") == null,
        };
    }

    pub fn deinit(self: *Store) void {
        self.parsed.deinit();
        self.allocator.free(self.path);
    }

    pub fn contains(self: *const Store, id: []const u8) bool {
        return self.parsed.value.object.get(id) != null;
    }

    pub fn remove(self: *Store, id: []const u8) !bool {
        if (!self.parsed.value.object.swapRemove(id)) return false;
        try self.save();
        return true;
    }

    pub fn get(self: *const Store, id: []const u8) ?[]const u8 {
        const value = self.parsed.value.object.get(id) orelse return null;
        return switch (value) {
            .string => |secret| secret,
            .object => |credential| switch (credential.get("access") orelse return null) {
                .string => |access_value| access_value,
                else => null,
            },
            else => null,
        };
    }

    pub fn access(self: *Store, id: []const u8) ![]const u8 {
        const value = self.parsed.value.object.get(id) orelse return error.CredentialMissing;
        if (value == .string) return value.string;
        const credential = switch (value) {
            .object => |object| object,
            else => return error.InvalidCredential,
        };
        const expires = switch (credential.get("expires") orelse return error.InvalidCredential) {
            .integer => |number| number,
            else => return error.InvalidCredential,
        };
        if (expires == std.math.maxInt(i64) or expires > std.Io.Clock.real.now(self.io).toSeconds() + 60)
            return self.get(id) orelse return error.InvalidCredential;
        const refresh_token = switch (credential.get("refresh") orelse return error.InvalidCredential) {
            .string => |string| string,
            else => return error.InvalidCredential,
        };
        const refreshed = try oauth.refresh(self.allocator, self.io, id, refresh_token);
        defer refreshed.deinit(self.allocator);
        try self.putOAuth(id, refreshed.access, refreshed.refresh, refreshed.expires, refreshed.account_id);
        return self.get(id) orelse return error.InvalidCredential;
    }

    pub fn getField(self: *const Store, id: []const u8, field: []const u8) ?[]const u8 {
        const credential = switch (self.parsed.value.object.get(id) orelse return null) {
            .object => |object| object,
            else => return null,
        };
        return switch (credential.get(field) orelse return null) {
            .string => |value| value,
            else => null,
        };
    }

    pub fn put(self: *Store, id: []const u8, secret: []const u8) !void {
        if (id.len == 0 or secret.len == 0 or std.mem.indexOfScalar(u8, id, 0) != null) return error.InvalidCredential;
        const arena = self.parsed.arena.allocator();
        try self.parsed.value.object.put(arena, try arena.dupe(u8, id), .{ .string = try arena.dupe(u8, secret) });
        try self.save();
    }

    pub fn putOAuth(self: *Store, id: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8) !void {
        const arena = self.parsed.arena.allocator();
        var credential: std.json.ObjectMap = .{};
        try credential.put(arena, "type", .{ .string = "oauth" });
        try credential.put(arena, "access", .{ .string = try arena.dupe(u8, access_value) });
        try credential.put(arena, "refresh", .{ .string = try arena.dupe(u8, refresh_value) });
        try credential.put(arena, "expires", .{ .integer = expires });
        if (account_id) |value| try credential.put(arena, "account_id", .{ .string = try arena.dupe(u8, value) });
        try self.parsed.value.object.put(arena, try arena.dupe(u8, id), .{ .object = credential });
        try self.save();
    }

    fn save(self: *Store) !void {
        const document = try std.json.Stringify.valueAlloc(self.allocator, self.parsed.value, .{ .whitespace = .indent_2 });
        defer self.allocator.free(document);
        const directory = std.fs.path.dirname(self.path) orelse ".";
        try std.Io.Dir.cwd().createDirPath(self.io, directory);
        if (self.protect_parent) {
            var credential_dir = try std.Io.Dir.cwd().openDir(self.io, directory, .{ .iterate = true });
            defer credential_dir.close(self.io);
            try credential_dir.setPermissions(self.io, @enumFromInt(0o700));
        }
        var atomic = try std.Io.Dir.cwd().createFileAtomic(self.io, self.path, .{
            .permissions = @enumFromInt(0o600),
            .make_path = true,
            .replace = true,
        });
        defer atomic.deinit(self.io);
        try atomic.file.writeStreamingAll(self.io, document);
        try atomic.file.sync(self.io);
        try atomic.replace(self.io);
    }
};

pub const Action = enum { login, logout, status };

pub fn command(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: Action, provider: []const u8) !bool {
    if (std.mem.eql(u8, provider, "claude")) {
        try claudeAuth(io, @tagName(action));
        return action != .logout;
    }
    if (!managedProvider(provider)) return error.UnknownProvider;
    var store = try Store.init(allocator, io, environ);
    defer store.deinit();
    switch (action) {
        .status => return store.contains(provider),
        .logout => {
            _ = try store.remove(provider);
            return false;
        },
        .login => {},
    }
    if (std.mem.eql(u8, provider, "openai-codex") or std.mem.eql(u8, provider, "kimi-coding")) {
        const credential = if (std.mem.eql(u8, provider, "openai-codex"))
            try oauth.loginOpenAI(allocator, io)
        else
            try oauth.loginKimi(allocator, io);
        defer credential.deinit(allocator);
        try store.putOAuth(provider, credential.access, credential.refresh, credential.expires, credential.account_id);
    } else if (std.mem.eql(u8, provider, "openrouter")) {
        const authorization = try oauth.startOpenRouter(allocator, io);
        defer authorization.deinit(allocator);
        std.debug.print("Open this URL:\n{s}\n", .{authorization.url});
        const input = try readSecret(allocator, io, "Paste the authorization code or redirect URL: ");
        defer allocator.free(input);
        const credential = try oauth.finishOpenRouter(allocator, io, authorization.verifier, input);
        defer credential.deinit(allocator);
        try store.putOAuth(provider, credential.access, credential.refresh, credential.expires, credential.account_id);
    } else {
        const secret = try readSecret(allocator, io, "API key: ");
        defer allocator.free(secret);
        try store.put(provider, secret);
    }
    std.debug.print("misa: saved {s} credential to {s}\n", .{ provider, store.path });
    return true;
}

fn managedProvider(provider: []const u8) bool {
    return std.mem.eql(u8, provider, "openai") or std.mem.eql(u8, provider, "openai-codex") or
        std.mem.eql(u8, provider, "anthropic") or std.mem.eql(u8, provider, "openrouter") or
        std.mem.eql(u8, provider, "kimi-coding");
}

fn claudeAuth(io: std.Io, action: []const u8) !void {
    var child = try std.process.spawn(io, .{ .argv = &.{ "claude", "auth", action } });
    defer child.kill(io);
    const term = try child.wait(io);
    if (term != .exited or term.exited != 0) return error.ClaudeAuthFailed;
}

pub fn credentialPath(allocator: std.mem.Allocator, environ: *const std.process.Environ.Map) ![]u8 {
    if (environ.get("MISA_AUTH_FILE")) |path| return allocator.dupe(u8, path);
    if (environ.get("XDG_STATE_HOME")) |root| return std.fs.path.join(allocator, &.{ root, "misa", "auth.json" });
    if (environ.get("HOME")) |home| return std.fs.path.join(allocator, &.{ home, ".local", "state", "misa", "auth.json" });
    return error.CredentialPathUnavailable;
}

pub fn readSecret(allocator: std.mem.Allocator, io: std.Io, prompt: []const u8) ![]u8 {
    try std.Io.File.stderr().writeStreamingAll(io, prompt);
    const saved = posix.tcgetattr(posix.STDIN_FILENO) catch null;
    if (saved) |original| {
        var hidden = original;
        hidden.lflag.ECHO = false;
        try posix.tcsetattr(posix.STDIN_FILENO, .NOW, hidden);
    }
    defer if (saved) |original| {
        posix.tcsetattr(posix.STDIN_FILENO, .NOW, original) catch {};
        std.Io.File.stderr().writeStreamingAll(io, "\n") catch {};
    };

    var result: std.ArrayList(u8) = .empty;
    errdefer result.deinit(allocator);
    var byte: [1]u8 = undefined;
    while (true) {
        const count = std.Io.File.stdin().readStreaming(io, &.{&byte}) catch |err| switch (err) {
            error.EndOfStream => break,
            else => return err,
        };
        if (count == 0 or byte[0] == '\n' or byte[0] == '\r') break;
        if (byte[0] == 0 or byte[0] < 0x20 or byte[0] == 0x7f) continue;
        try result.append(allocator, byte[0]);
        if (result.items.len > 64 * 1024) return error.CredentialTooLong;
    }
    if (result.items.len == 0) return error.EmptyCredential;
    return result.toOwnedSlice(allocator);
}

test "credential paths follow explicit and XDG precedence" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("HOME", "/home/test");
    var path = try credentialPath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/home/test/.local/state/misa/auth.json", path);
    std.testing.allocator.free(path);
    try environ.put("XDG_STATE_HOME", "/state");
    path = try credentialPath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/state/misa/auth.json", path);
    std.testing.allocator.free(path);
    try environ.put("MISA_AUTH_FILE", "/secret/auth.json");
    path = try credentialPath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/secret/auth.json", path);
    std.testing.allocator.free(path);
}
