//! XDG credential storage and explicit login commands.
const std = @import("std");
const posix = std.posix;
pub const oauth = @import("oauth.zig");

// Every in-process credential read/refresh/mutation is serialized. Mutators
// reload after acquiring the lock, so two workers cannot publish stale maps.
var credential_mutex: std.Io.Mutex = .init;
const credential_lock_name = ".misa-auth.lock";

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
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        if (!self.parsed.value.object.swapRemove(id)) return false;
        try self.save(directory);
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
        const Refresh = struct { token: []u8, profile: ?[]u8, token_url: ?[]u8, api_base: ?[]u8, version: u64 };
        const refresh_data: Refresh = snapshot: {
            try credential_mutex.lock(self.io);
            defer credential_mutex.unlock(self.io);
            var directory = try self.prepareDirectory();
            defer directory.close(self.io);
            var lock = try lockDirectoryFile(self.io, directory);
            defer lock.close(self.io);
            try self.reload(directory);
            const value = self.parsed.value.object.get(id) orelse return error.CredentialMissing;
            if (value == .string) return value.string;
            const credential = if (value == .object) value.object else return error.InvalidCredential;
            const expires = if (credential.get("expires")) |v| if (v == .integer) v.integer else return error.InvalidCredential else return error.InvalidCredential;
            if (expires == std.math.maxInt(i64) or expires > std.Io.Clock.real.now(self.io).toSeconds() + 60)
                return self.get(id) orelse return error.InvalidCredential;
            const refresh_token = if (credential.get("refresh")) |v| if (v == .string) v.string else return error.InvalidCredential else return error.InvalidCredential;
            const token = try self.allocator.dupe(u8, refresh_token);
            errdefer self.allocator.free(token);
            const profile = if (self.getField(id, "profile")) |v| try self.allocator.dupe(u8, v) else null;
            errdefer if (profile) |v| self.allocator.free(v);
            const token_url = if (self.getField(id, "token_url")) |v| try self.allocator.dupe(u8, v) else null;
            errdefer if (token_url) |v| self.allocator.free(v);
            const api_base = if (self.getField(id, "api_base")) |v| try self.allocator.dupe(u8, v) else null;
            break :snapshot .{ .token = token, .profile = profile, .token_url = token_url, .api_base = api_base, .version = try credentialVersion(credential) };
        };
        defer self.allocator.free(refresh_data.token);
        defer if (refresh_data.profile) |v| self.allocator.free(v);
        defer if (refresh_data.token_url) |v| self.allocator.free(v);
        defer if (refresh_data.api_base) |v| self.allocator.free(v);

        // Refresh can block on DNS/network and must never own either lock.
        const refreshed = try oauth.refresh(self.allocator, self.io, id, refresh_data.token, refresh_data.token_url);
        defer refreshed.deinit(self.allocator);
        return self.publishOAuthRefresh(id, refresh_data.token, refresh_data.profile, refresh_data.version, refreshed.access, refreshed.refresh, refreshed.expires, refreshed.account_id, refresh_data.token_url, refresh_data.api_base);
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
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        const arena = self.parsed.arena.allocator();
        try self.parsed.value.object.put(arena, try arena.dupe(u8, id), .{ .string = try arena.dupe(u8, secret) });
        try self.save(directory);
    }

    pub fn putOAuth(self: *Store, id: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8) !void {
        return self.putOAuthMetadata(id, access_value, refresh_value, expires, account_id, null, null, null);
    }

    pub fn putOAuthMetadata(self: *Store, id: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8, profile: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8) !void {
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        const old_version = if (self.parsed.value.object.get(id)) |value| switch (value) {
            .object => |credential| try credentialVersion(credential),
            else => 0,
        } else 0;
        if (old_version == std.math.maxInt(i64)) return error.InvalidCredential;
        return self.putOAuthMetadataUnlocked(directory, id, access_value, refresh_value, expires, account_id, profile, token_url, api_base, old_version + 1);
    }

    /// Publish a network refresh only if the exact credential generation used
    /// to obtain it still owns this provider. The reload and comparison happen
    /// under both locks, so logout and a replacement login always win.
    fn publishOAuthRefresh(self: *Store, id: []const u8, expected_refresh: []const u8, expected_profile: ?[]const u8, expected_version: u64, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8) ![]const u8 {
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);

        const current = self.parsed.value.object.get(id) orelse return error.CredentialMissing;
        if (current != .object) return currentAccess(current);
        const credential = current.object;
        const current_refresh = optionalString(credential, "refresh") orelse return currentAccess(current);
        const current_profile = optionalString(credential, "profile");
        const current_version = credentialVersion(credential) catch return currentAccess(current);
        if (!std.mem.eql(u8, current_refresh, expected_refresh) or !optionalEql(current_profile, expected_profile) or current_version != expected_version)
            return currentAccess(current);

        if (expected_version == std.math.maxInt(i64)) return error.InvalidCredential;
        try self.putOAuthMetadataUnlocked(directory, id, access_value, refresh_value, expires, account_id, expected_profile, token_url, api_base, expected_version + 1);
        return self.get(id) orelse return error.InvalidCredential;
    }

    fn putOAuthMetadataUnlocked(self: *Store, directory: std.Io.Dir, id: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8, profile: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8, version: u64) !void {
        const arena = self.parsed.arena.allocator();
        var credential: std.json.ObjectMap = .{};
        try credential.put(arena, "type", .{ .string = "oauth" });
        try credential.put(arena, "access", .{ .string = try arena.dupe(u8, access_value) });
        try credential.put(arena, "refresh", .{ .string = try arena.dupe(u8, refresh_value) });
        try credential.put(arena, "expires", .{ .integer = expires });
        try credential.put(arena, "version", .{ .integer = @intCast(version) });
        if (account_id) |value| try credential.put(arena, "account_id", .{ .string = try arena.dupe(u8, value) });
        if (profile) |value| try credential.put(arena, "profile", .{ .string = try arena.dupe(u8, value) });
        if (token_url) |value| try credential.put(arena, "token_url", .{ .string = try arena.dupe(u8, value) });
        if (api_base) |value| try credential.put(arena, "api_base", .{ .string = try arena.dupe(u8, value) });
        try self.parsed.value.object.put(arena, try arena.dupe(u8, id), .{ .object = credential });
        try self.save(directory);
    }

    fn reload(self: *Store, directory: std.Io.Dir) !void {
        const source = directory.readFileAlloc(self.io, std.fs.path.basename(self.path), self.allocator, .limited(1024 * 1024)) catch |err| switch (err) {
            error.FileNotFound => try self.allocator.dupe(u8, "{}"),
            else => return err,
        };
        defer self.allocator.free(source);
        var replacement = try std.json.parseFromSlice(std.json.Value, self.allocator, source, .{ .allocate = .alloc_always });
        errdefer replacement.deinit();
        if (replacement.value != .object) return error.InvalidCredentialStore;
        self.parsed.deinit();
        self.parsed = replacement;
    }

    fn prepareDirectory(self: *Store) !std.Io.Dir {
        const path = std.fs.path.dirname(self.path) orelse ".";
        try std.Io.Dir.cwd().createDirPath(self.io, path);
        var directory = try std.Io.Dir.cwd().openDir(self.io, path, .{ .iterate = true, .follow_symlinks = false });
        errdefer directory.close(self.io);
        if (self.protect_parent) try directory.setPermissions(self.io, @enumFromInt(0o700));
        return directory;
    }

    fn save(self: *Store, directory: std.Io.Dir) !void {
        const document = try std.json.Stringify.valueAlloc(self.allocator, self.parsed.value, .{ .whitespace = .indent_2 });
        defer self.allocator.free(document);
        var atomic = try directory.createFileAtomic(self.io, std.fs.path.basename(self.path), .{
            .permissions = @enumFromInt(0o600),
            .replace = true,
        });
        defer atomic.deinit(self.io);
        try atomic.file.writeStreamingAll(self.io, document);
        try atomic.file.sync(self.io);
        try atomic.replace(self.io);
        const directory_file: std.Io.File = .{ .handle = directory.handle, .flags = .{ .nonblocking = false } };
        try directory_file.sync(self.io);
    }
};

fn optionalString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return if (value == .string) value.string else null;
}

fn optionalEql(a: ?[]const u8, b: ?[]const u8) bool {
    if (a == null or b == null) return a == null and b == null;
    return std.mem.eql(u8, a.?, b.?);
}

// Stores written before credential generations were introduced are generation
// zero. Every native OAuth mutation writes and increments the generation.
fn credentialVersion(credential: std.json.ObjectMap) !u64 {
    const value = credential.get("version") orelse return 0;
    if (value != .integer or value.integer < 0) return error.InvalidCredential;
    return @intCast(value.integer);
}

fn currentAccess(value: std.json.Value) ![]const u8 {
    return switch (value) {
        .string => |secret| secret,
        .object => |credential| optionalString(credential, "access") orelse error.CredentialMissing,
        else => error.CredentialMissing,
    };
}

fn lockDirectoryFile(io: std.Io, directory: std.Io.Dir) !std.Io.File {
    while (true) {
        return directory.openFile(io, credential_lock_name, .{
            .mode = .read_write,
            .allow_directory = false,
            .follow_symlinks = false,
            .resolve_beneath = true,
            .lock = .exclusive,
        }) catch |err| switch (err) {
            error.FileNotFound => directory.createFile(io, credential_lock_name, .{
                .read = true,
                .truncate = false,
                .exclusive = true,
                .permissions = @enumFromInt(0o600),
                .resolve_beneath = true,
                .lock = .exclusive,
            }) catch |create_err| switch (create_err) {
                error.PathAlreadyExists => continue,
                else => return create_err,
            },
            else => return err,
        };
    }
}

/// Bind standard credentials to native trusted origins. Additional origins can
/// only come from the user's process environment, never from an extension:
/// MISA_CREDENTIAL_ORIGINS='{"openai":["https://gateway.example"]}'.
pub fn validateCredentialOrigin(id: []const u8, url: []const u8, store: ?*Store, environ: *const std.process.Environ.Map) !void {
    // Subscription quota operations live outside the Codex responses prefix.
    // Grant only these exact endpoints, not the rest of ChatGPT's backend API.
    if (std.mem.eql(u8, id, "openai-codex") and
        (std.mem.eql(u8, url, "https://chatgpt.com/backend-api/wham/usage") or
            std.mem.eql(u8, url, "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits") or
            std.mem.eql(u8, url, "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume"))) return;
    if (std.mem.eql(u8, id, "kimi-coding")) {
        const credential_store = store orelse return error.CredentialStoreUnavailable;
        const api_base = credential_store.getField(id, "api_base") orelse return error.CredentialProfileMissing;
        if ((!std.mem.eql(u8, api_base, "https://api.kimi.ai/coding/v1") and !std.mem.eql(u8, api_base, "https://api.kimi.com/coding/v1")) or !urlUnder(url, api_base))
            return error.CredentialProfileMismatch;
        return;
    }
    const trusted: ?[]const u8 = if (std.mem.eql(u8, id, "openai"))
        "https://api.openai.com"
    else if (std.mem.eql(u8, id, "anthropic"))
        "https://api.anthropic.com"
    else if (std.mem.eql(u8, id, "openrouter"))
        "https://openrouter.ai"
    else if (std.mem.eql(u8, id, "openai-codex"))
        "https://chatgpt.com/backend-api/codex"
    else
        null;
    if (trusted) |origin| if (urlUnder(url, origin)) return;
    const grants_source = environ.get("MISA_CREDENTIAL_ORIGINS") orelse return error.CredentialOriginDenied;
    if (grants_source.len > 64 * 1024) return error.InvalidCredentialOriginGrant;
    var grants = std.json.parseFromSlice(std.json.Value, std.heap.page_allocator, grants_source, .{}) catch return error.InvalidCredentialOriginGrant;
    defer grants.deinit();
    const object = if (grants.value == .object) grants.value.object else return error.InvalidCredentialOriginGrant;
    const values = object.get(id) orelse return error.CredentialOriginDenied;
    if (values != .array) return error.InvalidCredentialOriginGrant;
    for (values.array.items) |value| {
        if (value != .string or !validGrant(value.string)) return error.InvalidCredentialOriginGrant;
        if (urlUnder(url, value.string)) return;
    }
    return error.CredentialOriginDenied;
}

fn validGrant(value: []const u8) bool {
    if (!std.mem.startsWith(u8, value, "https://") or value.len <= "https://".len or value[value.len - 1] == '/') return false;
    const rest = value["https://".len..];
    return std.mem.indexOfAny(u8, rest, "/?#\r\n\x00") == null;
}

fn urlUnder(url: []const u8, base: []const u8) bool {
    return std.mem.startsWith(u8, url, base) and (url.len == base.len or url[base.len] == '/' or url[base.len] == '?' or url[base.len] == '#');
}

pub const Action = enum { login, logout, status };
pub const Strategy = enum { api_key, cli_handoff, device_oauth, loopback_pkce };

/// Provider-owned transport declaration validated against native trusted endpoints.
pub const Declaration = struct {
    provider: []const u8,
    strategy: Strategy,
    profile_id: ?[]const u8 = null,
    authorization_url: ?[]const u8 = null,
    token_url: ?[]const u8 = null,
    api_base: ?[]const u8 = null,

    pub fn parse(provider: []const u8, strategy_name: []const u8, profile_value: std.json.Value) !Declaration {
        const strategy = std.meta.stringToEnum(Strategy, strategy_name) orelse return error.UntrustedAuthDeclaration;
        var result: Declaration = .{ .provider = provider, .strategy = strategy };
        if (profile_value == .object) {
            const object = profile_value.object;
            result.profile_id = objectString(object, "id");
            result.authorization_url = objectString(object, "authorization_url");
            result.token_url = objectString(object, "token_url");
            result.api_base = objectString(object, "api_base");
        } else if (profile_value != .null) return error.UntrustedAuthDeclaration;
        try result.validate();
        return result;
    }
    pub fn validate(self: Declaration) !void {
        if (std.mem.eql(u8, self.provider, "openai") or std.mem.eql(u8, self.provider, "anthropic")) {
            if (self.strategy != .api_key) return error.UntrustedAuthDeclaration;
        } else if (std.mem.eql(u8, self.provider, "claude")) {
            if (self.strategy != .cli_handoff) return error.UntrustedAuthDeclaration;
        } else if (std.mem.eql(u8, self.provider, "openrouter")) {
            if (self.strategy != .loopback_pkce or !optionalEqual(self.profile_id, "default")) return error.UntrustedAuthDeclaration;
        } else if (std.mem.eql(u8, self.provider, "openai-codex")) {
            if (self.strategy != .device_oauth or !optionalEqual(self.profile_id, "default") or !optionalEqual(self.authorization_url, "https://auth.openai.com/api/accounts/deviceauth/usercode") or !optionalEqual(self.token_url, "https://auth.openai.com/oauth/token")) return error.UntrustedAuthDeclaration;
        } else if (std.mem.eql(u8, self.provider, "kimi-coding")) {
            if (self.strategy != .device_oauth) return error.UntrustedAuthDeclaration;
            const global = optionalEqual(self.profile_id, "global") and optionalEqual(self.authorization_url, "https://auth.kimi.ai/api/oauth/device_authorization") and optionalEqual(self.token_url, "https://auth.kimi.ai/api/oauth/token") and optionalEqual(self.api_base, "https://api.kimi.ai/coding/v1");
            const mainland = optionalEqual(self.profile_id, "mainland") and optionalEqual(self.authorization_url, "https://auth.kimi.com/api/oauth/device_authorization") and optionalEqual(self.token_url, "https://auth.kimi.com/api/oauth/token") and optionalEqual(self.api_base, "https://api.kimi.com/coding/v1");
            if (!global and !mainland) return error.UntrustedAuthDeclaration;
        } else return error.UnknownProvider;
    }
};
fn objectString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return if (value == .string and value.string.len != 0) value.string else null;
}
fn optionalEqual(value: ?[]const u8, expected: []const u8) bool {
    return value != null and std.mem.eql(u8, value.?, expected);
}

pub const CommandResult = struct {
    logged_in: bool,
    subscription_type: ?[]u8 = null,

    pub fn deinit(self: CommandResult, allocator: std.mem.Allocator) void {
        if (self.subscription_type) |value| allocator.free(value);
    }
};

pub const Interaction = struct {
    context: *anyopaque,
    emitFn: *const fn (*anyopaque, oauth.Prompt) anyerror!void,
    inputFn: *const fn (*anyopaque, []const u8) anyerror![]const u8,
    protected_input: bool = false,
    pub fn emit(self: Interaction, prompt: oauth.Prompt) !void {
        try self.emitFn(self.context, prompt);
    }
    pub fn input(self: Interaction, correlation: []const u8) ![]const u8 {
        return self.inputFn(self.context, correlation);
    }
};

pub fn commandTerminal(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: Action, provider: []const u8) !CommandResult {
    const declaration: Declaration = if (std.mem.eql(u8, provider, "openai")) .{ .provider = provider, .strategy = .api_key } else if (std.mem.eql(u8, provider, "anthropic")) .{ .provider = provider, .strategy = .api_key } else if (std.mem.eql(u8, provider, "claude")) .{ .provider = provider, .strategy = .cli_handoff } else if (std.mem.eql(u8, provider, "openrouter")) .{ .provider = provider, .strategy = .loopback_pkce, .profile_id = "default" } else if (std.mem.eql(u8, provider, "openai-codex")) .{ .provider = provider, .strategy = .device_oauth, .profile_id = "default", .authorization_url = "https://auth.openai.com/api/accounts/deviceauth/usercode", .token_url = "https://auth.openai.com/oauth/token" } else if (std.mem.eql(u8, provider, "kimi-coding")) .{ .provider = provider, .strategy = .device_oauth, .profile_id = "global", .authorization_url = "https://auth.kimi.ai/api/oauth/device_authorization", .token_url = "https://auth.kimi.ai/api/oauth/token", .api_base = "https://api.kimi.ai/coding/v1" } else return error.UnknownProvider;
    var context = TerminalInteraction{ .allocator = allocator, .io = io };
    defer if (context.owned_input) |value| {
        std.crypto.secureZero(u8, value);
        allocator.free(value);
    };
    return command(allocator, io, environ, action, declaration, .{ .context = &context, .emitFn = TerminalInteraction.emit, .inputFn = TerminalInteraction.input });
}
const TerminalInteraction = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    owned_input: ?[]u8 = null,
    fn emit(context: *anyopaque, prompt: oauth.Prompt) !void {
        const self: *TerminalInteraction = @ptrCast(@alignCast(context));
        const text = try std.fmt.allocPrint(self.allocator, "{s}\n{s}\n{s}\n", .{ prompt.message, prompt.url orelse "", prompt.code orelse "" });
        defer self.allocator.free(text);
        try std.Io.File.stderr().writeStreamingAll(self.io, text);
    }
    fn input(context: *anyopaque, _: []const u8) ![]const u8 {
        const self: *TerminalInteraction = @ptrCast(@alignCast(context));
        self.owned_input = try readSecret(self.allocator, self.io, "Paste authorization code or URL: ");
        return self.owned_input.?;
    }
};

pub fn command(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: Action, declaration: Declaration, interaction: Interaction) !CommandResult {
    try declaration.validate();
    const provider = declaration.provider;
    if (std.mem.eql(u8, provider, "claude")) {
        return commandCli(allocator, io, environ, action, "claude");
    }
    if (!managedProvider(provider)) return error.UnknownProvider;
    var store = try Store.init(allocator, io, environ);
    defer store.deinit();
    switch (action) {
        .status => return .{ .logged_in = store.contains(provider) },
        .logout => {
            _ = try store.remove(provider);
            return .{ .logged_in = false };
        },
        .login => {},
    }
    if (std.mem.eql(u8, provider, "openai-codex") or std.mem.eql(u8, provider, "kimi-coding")) {
        const credential = if (std.mem.eql(u8, provider, "openai-codex"))
            try oauth.loginOpenAI(allocator, io, interaction)
        else
            try oauth.loginKimi(allocator, io, declaration.authorization_url.?, declaration.token_url.?, interaction);
        defer credential.deinit(allocator);
        try store.putOAuthMetadata(provider, credential.access, credential.refresh, credential.expires, credential.account_id, declaration.profile_id, declaration.token_url, declaration.api_base);
    } else if (std.mem.eql(u8, provider, "openrouter")) {
        const authorization = try oauth.startOpenRouter(allocator, io);
        defer authorization.deinit(allocator);
        const input = try oauth.authorizeOpenRouter(allocator, io, authorization, interaction);
        defer allocator.free(input);
        const credential = try oauth.finishOpenRouter(allocator, io, authorization.verifier, input);
        defer credential.deinit(allocator);
        try store.putOAuth(provider, credential.access, credential.refresh, credential.expires, credential.account_id);
    } else {
        if (interaction.protected_input) {
            try interaction.emit(.{ .correlation = "api-key", .kind = "modal", .title = "API key", .message = "Enter your API key. Characters are hidden.", .input = true, .protected = true });
            const secret = try interaction.input("api-key");
            if (secret.len == 0) return error.EmptyCredential;
            try store.put(provider, secret);
        } else {
            const secret = try readSecret(allocator, io, "API key: ");
            defer {
                std.crypto.secureZero(u8, secret);
                allocator.free(secret);
            }
            try store.put(provider, secret);
        }
    }
    if (!interaction.protected_input) std.debug.print("misa: saved {s} credential to {s}\n", .{ provider, store.path });
    return .{ .logged_in = true };
}

fn managedProvider(provider: []const u8) bool {
    return std.mem.eql(u8, provider, "openai") or std.mem.eql(u8, provider, "openai-codex") or
        std.mem.eql(u8, provider, "anthropic") or std.mem.eql(u8, provider, "openrouter") or
        std.mem.eql(u8, provider, "kimi-coding");
}

/// CLI adapter takes its executable explicitly; fixture composition supplies
/// its registered script without resolving an installed provider command.
pub fn commandCli(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: Action, executable: []const u8) !CommandResult {
    if (action == .status) return claudeStatus(allocator, io, environ, executable);
    try claudeAuth(allocator, io, environ, executable, @tagName(action));
    return .{ .logged_in = action != .logout };
}

fn claudeStatus(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, executable: []const u8) !CommandResult {
    const captured = try std.process.run(allocator, io, .{
        .argv = &.{ executable, "auth", "status" },
        .environ_map = environ,
        .stdout_limit = .limited(64 * 1024),
        .stderr_limit = .limited(64 * 1024),
    });
    defer allocator.free(captured.stdout);
    defer allocator.free(captured.stderr);
    if (captured.term != .exited or captured.term.exited != 0) return error.ClaudeAuthFailed;
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, captured.stdout, .{});
    defer parsed.deinit();
    const object = switch (parsed.value) {
        .object => |value| value,
        else => return error.InvalidClaudeAuthStatus,
    };
    const logged_in = switch (object.get("loggedIn") orelse return error.InvalidClaudeAuthStatus) {
        .bool => |value| value,
        else => return error.InvalidClaudeAuthStatus,
    };
    const subscription = switch (object.get("subscriptionType") orelse .null) {
        .string => |value| try allocator.dupe(u8, value),
        .null => null,
        else => return error.InvalidClaudeAuthStatus,
    };
    return .{ .logged_in = logged_in, .subscription_type = subscription };
}

fn claudeAuth(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, executable: []const u8, action: []const u8) !void {
    if (std.mem.eql(u8, action, "logout")) {
        const result = try std.process.run(allocator, io, .{
            .argv = &.{ executable, "auth", action },
            .environ_map = environ,
            .stdout_limit = .limited(64 * 1024),
            .stderr_limit = .limited(64 * 1024),
        });
        defer allocator.free(result.stdout);
        defer allocator.free(result.stderr);
        if (result.term != .exited or result.term.exited != 0) return error.ClaudeAuthFailed;
        return;
    }
    var child = try std.process.spawn(io, .{ .argv = &.{ executable, "auth", action }, .environ_map = environ });
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

test "trusted Kimi regions are exact endpoint bundles" {
    const global: Declaration = .{ .provider = "kimi-coding", .strategy = .device_oauth, .profile_id = "global", .authorization_url = "https://auth.kimi.ai/api/oauth/device_authorization", .token_url = "https://auth.kimi.ai/api/oauth/token", .api_base = "https://api.kimi.ai/coding/v1" };
    try global.validate();
    const mainland: Declaration = .{ .provider = "kimi-coding", .strategy = .device_oauth, .profile_id = "mainland", .authorization_url = "https://auth.kimi.com/api/oauth/device_authorization", .token_url = "https://auth.kimi.com/api/oauth/token", .api_base = "https://api.kimi.com/coding/v1" };
    try mainland.validate();
    var drifted = global;
    drifted.token_url = "https://evil.example/token";
    try std.testing.expectError(error.UntrustedAuthDeclaration, drifted.validate());
}

test "credential stores reload under mutation lock to avoid lost updates" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/auth.json", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_AUTH_FILE", path);
    var first = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer first.deinit();
    var second = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer second.deinit();
    try first.put("openai", "one");
    try second.put("anthropic", "two");
    var final = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer final.deinit();
    try std.testing.expect(final.contains("openai"));
    try std.testing.expect(final.contains("anthropic"));
}

test "refresh compare-and-swap cannot resurrect a logged-out credential" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/auth.json", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_AUTH_FILE", path);
    var refreshing = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer refreshing.deinit();
    var logout = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer logout.deinit();
    try refreshing.putOAuthMetadata("openai-codex", "old-access", "old-refresh", 0, null, "default", "https://auth.example/token", null);
    try std.testing.expect(try logout.remove("openai-codex"));
    try std.testing.expectError(error.CredentialMissing, refreshing.publishOAuthRefresh("openai-codex", "old-refresh", "default", 1, "stale-access", "stale-refresh", 100, null, "https://auth.example/token", null));
    try std.testing.expect(!refreshing.contains("openai-codex"));
}

test "refresh compare-and-swap returns replacement login even when token and profile repeat" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/auth.json", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_AUTH_FILE", path);
    var refreshing = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer refreshing.deinit();
    var login = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer login.deinit();
    try refreshing.putOAuthMetadata("openai-codex", "old-access", "same-refresh", 0, null, "default", null, null);
    try login.putOAuthMetadata("openai-codex", "new-login-access", "same-refresh", 100, null, "default", null, null);
    const access_value = try refreshing.publishOAuthRefresh("openai-codex", "same-refresh", "default", 1, "stale-access", "stale-refresh", 200, null, null, null);
    try std.testing.expectEqualStrings("new-login-access", access_value);
    try std.testing.expectEqualStrings("same-refresh", refreshing.getField("openai-codex", "refresh").?);
}

test "standard credential origins require native trust or explicit user grant" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try validateCredentialOrigin("openai", "https://api.openai.com/v1/responses", null, &environ);
    try std.testing.expectError(error.CredentialOriginDenied, validateCredentialOrigin("openai", "https://evil.example/collect", null, &environ));
    try environ.put("MISA_CREDENTIAL_ORIGINS", "{\"openai\":[\"https://gateway.example\"]}");
    try validateCredentialOrigin("openai", "https://gateway.example/v1", null, &environ);
    try std.testing.expectError(error.CredentialOriginDenied, validateCredentialOrigin("openai", "https://gateway.example.evil/v1", null, &environ));
}

test "Codex usage trust does not grant the surrounding backend API" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try validateCredentialOrigin("openai-codex", "https://chatgpt.com/backend-api/wham/usage", null, &environ);
    for ([_][]const u8{
        "http://chatgpt.com/backend-api/wham/usage",
        "https://chatgpt.com.evil/backend-api/wham/usage",
        "https://chatgpt.com/backend-api/wham/usage/other",
        "https://chatgpt.com/backend-api/wham/usage-other",
        "https://chatgpt.com/backend-api/wham/other",
        "https://chatgpt.com/backend-api/other",
    }) |url| {
        try std.testing.expectError(error.CredentialOriginDenied, validateCredentialOrigin("openai-codex", url, null, &environ));
    }
    try std.testing.expectError(error.CredentialOriginDenied, validateCredentialOrigin("openai", "https://chatgpt.com/backend-api/wham/usage", null, &environ));
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
