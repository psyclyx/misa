//! XDG credential storage and explicit login commands.
const std = @import("std");
const posix = std.posix;
pub const oauth = @import("oauth.zig");

// Every in-process credential read/refresh/mutation is serialized. Mutators
// reload after acquiring the lock, so two workers cannot publish stale maps.
var credential_mutex: std.Io.Mutex = .init;
const credential_lock_name = ".misa-auth.lock";

/// One provider entry owns named accounts, so several logins can be held for
/// the same provider. Documents written before accounts existed stored one
/// credential per provider ID; that credential becomes the `default` account.
pub const default_account = "default";
const accounts_field = "accounts";
const active_field = "active";

/// An account name is a stable, printable key: anything else would be
/// ambiguous in the store or on a command line.
pub fn validAccountName(name: []const u8) bool {
    if (name.len == 0 or name.len > 64) return false;
    for (name) |byte| {
        if (!std.ascii.isAlphanumeric(byte) and byte != '-' and byte != '_' and byte != '.') return false;
    }
    return true;
}

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
        var store: Store = .{
            .allocator = allocator,
            .io = io,
            .path = path,
            .parsed = parsed,
            .protect_parent = environ.get("MISA_AUTH_FILE") == null,
        };
        try store.normalize();
        return store;
    }

    pub fn deinit(self: *Store) void {
        self.parsed.deinit();
        self.allocator.free(self.path);
    }

    /// Canonicalize every provider entry to `{active, accounts}` in memory. A
    /// document written before accounts existed keeps its credential as the
    /// `default` account. Nothing is written until the next mutation.
    fn normalize(self: *Store) !void {
        const arena = self.parsed.arena.allocator();
        var iterator = self.parsed.value.object.iterator();
        while (iterator.next()) |entry| {
            entry.value_ptr.* = try normalizeProvider(arena, entry.value_ptr.*);
        }
    }

    /// A provider entry is an object carrying an `accounts` object. Anything
    /// else is a credential from the single-account format.
    fn providerEntry(self: *const Store, provider: []const u8) ?std.json.Value {
        const value = self.parsed.value.object.get(provider) orelse return null;
        if (value != .object) return null;
        const accounts = value.object.get(accounts_field) orelse return null;
        return if (accounts == .object) value else null;
    }

    fn providerAccounts(self: *const Store, provider: []const u8) ?std.json.ObjectMap {
        const entry = self.providerEntry(provider) orelse return null;
        return entry.object.get(accounts_field).?.object;
    }

    /// The account this provider's requests use. A provider with no stored
    /// account has none.
    pub fn activeAccount(self: *const Store, provider: []const u8) ?[]const u8 {
        const accounts = self.providerAccounts(provider) orelse return null;
        if (accounts.count() == 0) return null;
        const active = self.providerEntry(provider).?.object.get(active_field) orelse return null;
        if (active != .string) return null;
        return if (accounts.get(active.string) != null) active.string else null;
    }

    pub fn contains(self: *const Store, provider: []const u8) bool {
        return self.activeAccount(provider) != null;
    }

    pub fn containsAccount(self: *const Store, provider: []const u8, account: []const u8) bool {
        const accounts = self.providerAccounts(provider) orelse return false;
        return accounts.get(account) != null;
    }

    /// Stored account names in presentation order: `default` first, then
    /// ascending. The caller owns the array and every name inside it.
    pub fn accountNames(self: *const Store, allocator: std.mem.Allocator, provider: []const u8) ![][]u8 {
        const accounts = self.providerAccounts(provider) orelse return allocator.alloc([]u8, 0);
        var names: std.ArrayList([]u8) = .empty;
        errdefer {
            for (names.items) |name| allocator.free(name);
            names.deinit(allocator);
        }
        var iterator = accounts.iterator();
        while (iterator.next()) |entry| try names.append(allocator, try allocator.dupe(u8, entry.key_ptr.*));
        std.mem.sort([]u8, names.items, {}, accountBefore);
        return names.toOwnedSlice(allocator);
    }

    /// Remove every account of a provider.
    pub fn remove(self: *Store, provider: []const u8) !bool {
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        if (!self.parsed.value.object.swapRemove(provider)) return false;
        try self.save(directory);
        return true;
    }

    /// Remove one account. The provider disappears with its last account.
    pub fn removeAccount(self: *Store, provider: []const u8, account: []const u8) !bool {
        if (!validAccountName(account)) return error.InvalidCredential;
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        const entry = self.parsed.value.object.getPtr(provider) orelse return false;
        if (self.providerAccounts(provider) == null) return false;
        const accounts = &entry.object.getPtr(accounts_field).?.object;
        if (!accounts.swapRemove(account)) return false;
        const arena = self.parsed.arena.allocator();
        if (accounts.count() == 0) {
            _ = self.parsed.value.object.swapRemove(provider);
        } else try repairActive(arena, &entry.object);
        try self.save(directory);
        return true;
    }

    /// Choose which stored account this provider's requests use. Selecting an
    /// account that is not stored reports false instead of inventing one.
    pub fn select(self: *Store, provider: []const u8, account: []const u8) !bool {
        if (!validAccountName(account)) return error.InvalidCredential;
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        if (!self.containsAccount(provider, account)) return false;
        if (self.activeAccount(provider)) |active| if (std.mem.eql(u8, active, account)) return true;
        try self.setActive(provider, account);
        try self.save(directory);
        return true;
    }

    pub fn getAccount(self: *const Store, provider: []const u8, account: []const u8) ?[]const u8 {
        const accounts = self.providerAccounts(provider) orelse return null;
        return secretOf(accounts.get(account) orelse return null);
    }

    pub fn get(self: *const Store, provider: []const u8) ?[]const u8 {
        return self.getAccount(provider, self.activeAccount(provider) orelse return null);
    }

    pub fn getAccountField(self: *const Store, provider: []const u8, account: []const u8, field: []const u8) ?[]const u8 {
        const accounts = self.providerAccounts(provider) orelse return null;
        const credential = switch (accounts.get(account) orelse return null) {
            .object => |object| object,
            else => return null,
        };
        return switch (credential.get(field) orelse return null) {
            .string => |value| value,
            else => null,
        };
    }

    pub fn getField(self: *const Store, provider: []const u8, field: []const u8) ?[]const u8 {
        return self.getAccountField(provider, self.activeAccount(provider) orelse return null, field);
    }

    fn accountVersion(self: *const Store, provider: []const u8, account: []const u8) !u64 {
        const accounts = self.providerAccounts(provider) orelse return 0;
        const value = accounts.get(account) orelse return 0;
        return switch (value) {
            .object => |credential| credentialVersion(credential),
            else => 0,
        };
    }

    pub fn access(self: *Store, provider: []const u8) ![]const u8 {
        const Refresh = struct { account: []u8, token: []u8, profile: ?[]u8, token_url: ?[]u8, api_base: ?[]u8, version: u64 };
        const refresh_data: Refresh = snapshot: {
            try credential_mutex.lock(self.io);
            defer credential_mutex.unlock(self.io);
            var directory = try self.prepareDirectory();
            defer directory.close(self.io);
            var lock = try lockDirectoryFile(self.io, directory);
            defer lock.close(self.io);
            try self.reload(directory);
            const account = self.activeAccount(provider) orelse return error.CredentialMissing;
            const value = self.providerAccounts(provider).?.get(account) orelse return error.CredentialMissing;
            if (value == .string) return value.string;
            const credential = if (value == .object) value.object else return error.InvalidCredential;
            if (optionalString(credential, "type")) |credential_type|
                if (std.mem.eql(u8, credential_type, "api_key")) return self.get(provider) orelse return error.InvalidCredential;
            const expires = if (credential.get("expires")) |v| if (v == .integer) v.integer else return error.InvalidCredential else return error.InvalidCredential;
            if (expires == std.math.maxInt(i64) or expires > std.Io.Clock.real.now(self.io).toSeconds() + 60)
                return self.get(provider) orelse return error.InvalidCredential;
            const refresh_token = if (credential.get("refresh")) |v| if (v == .string) v.string else return error.InvalidCredential else return error.InvalidCredential;
            // The account is resolved once: a concurrent switch cannot move an
            // in-flight refresh onto a credential it did not read.
            const account_name = try self.allocator.dupe(u8, account);
            errdefer self.allocator.free(account_name);
            const token = try self.allocator.dupe(u8, refresh_token);
            errdefer self.allocator.free(token);
            const profile = if (self.getAccountField(provider, account, "profile")) |v| try self.allocator.dupe(u8, v) else null;
            errdefer if (profile) |v| self.allocator.free(v);
            const token_url = if (self.getAccountField(provider, account, "token_url")) |v| try self.allocator.dupe(u8, v) else null;
            errdefer if (token_url) |v| self.allocator.free(v);
            const api_base = if (self.getAccountField(provider, account, "api_base")) |v| try self.allocator.dupe(u8, v) else null;
            break :snapshot .{ .account = account_name, .token = token, .profile = profile, .token_url = token_url, .api_base = api_base, .version = try credentialVersion(credential) };
        };
        defer self.allocator.free(refresh_data.account);
        defer self.allocator.free(refresh_data.token);
        defer if (refresh_data.profile) |v| self.allocator.free(v);
        defer if (refresh_data.token_url) |v| self.allocator.free(v);
        defer if (refresh_data.api_base) |v| self.allocator.free(v);

        // Refresh can block on DNS/network and must never own either lock.
        const refreshed = try oauth.refresh(self.allocator, self.io, provider, refresh_data.token, refresh_data.token_url, refresh_data.profile);
        defer refreshed.deinit(self.allocator);
        return self.publishOAuthRefresh(provider, refresh_data.account, refresh_data.token, refresh_data.profile, refresh_data.version, refreshed.access, refreshed.refresh, refreshed.expires, refreshed.account_id, refresh_data.token_url, refresh_data.api_base);
    }

    pub fn put(self: *Store, provider: []const u8, secret: []const u8) !void {
        return self.putAccount(provider, self.activeAccount(provider) orelse default_account, secret);
    }

    pub fn putAccount(self: *Store, provider: []const u8, account: []const u8, secret: []const u8) !void {
        if (provider.len == 0 or secret.len == 0 or std.mem.indexOfScalar(u8, provider, 0) != null) return error.InvalidCredential;
        if (!validAccountName(account)) return error.InvalidCredential;
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        const arena = self.parsed.arena.allocator();
        var credential: std.json.ObjectMap = .{};
        try credential.put(arena, "type", .{ .string = "api_key" });
        try credential.put(arena, "access", .{ .string = try arena.dupe(u8, secret) });
        try self.commitCredential(directory, provider, account, .{ .object = credential }, true);
    }

    pub fn putApiKeyMetadata(self: *Store, provider: []const u8, secret: []const u8, api_base: []const u8) !void {
        return self.putAccountApiKeyMetadata(provider, self.activeAccount(provider) orelse default_account, secret, api_base);
    }

    pub fn putAccountApiKeyMetadata(self: *Store, provider: []const u8, account: []const u8, secret: []const u8, api_base: []const u8) !void {
        if (provider.len == 0 or secret.len == 0 or api_base.len == 0 or std.mem.indexOfScalar(u8, provider, 0) != null) return error.InvalidCredential;
        if (!validAccountName(account)) return error.InvalidCredential;
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        const arena = self.parsed.arena.allocator();
        var credential: std.json.ObjectMap = .{};
        try credential.put(arena, "type", .{ .string = "api_key" });
        try credential.put(arena, "access", .{ .string = try arena.dupe(u8, secret) });
        try credential.put(arena, "api_base", .{ .string = try arena.dupe(u8, api_base) });
        try self.commitCredential(directory, provider, account, .{ .object = credential }, true);
    }

    pub fn putOAuth(self: *Store, provider: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8) !void {
        return self.putOAuthMetadata(provider, access_value, refresh_value, expires, account_id, null, null, null);
    }

    pub fn putOAuthMetadata(self: *Store, provider: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8, profile: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8) !void {
        return self.putOAuthMetadataAccount(provider, self.activeAccount(provider) orelse default_account, access_value, refresh_value, expires, account_id, profile, token_url, api_base);
    }

    pub fn putOAuthMetadataAccount(self: *Store, provider: []const u8, account: []const u8, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8, profile: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8) !void {
        if (!validAccountName(account)) return error.InvalidCredential;
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        const old_version = try self.accountVersion(provider, account);
        if (old_version == std.math.maxInt(i64)) return error.InvalidCredential;
        try self.writeOAuth(directory, provider, account, .{
            .access = access_value,
            .refresh = refresh_value,
            .expires = expires,
            .account_id = account_id,
            .profile = profile,
            .token_url = token_url,
            .api_base = api_base,
        }, old_version + 1, true);
    }

    const OAuthFields = struct { access: []const u8, refresh: []const u8, expires: i64, account_id: ?[]const u8, profile: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8 };

    fn writeOAuth(self: *Store, directory: std.Io.Dir, provider: []const u8, account: []const u8, fields: OAuthFields, version: u64, use: bool) !void {
        const arena = self.parsed.arena.allocator();
        var credential: std.json.ObjectMap = .{};
        try credential.put(arena, "type", .{ .string = "oauth" });
        try credential.put(arena, "access", .{ .string = try arena.dupe(u8, fields.access) });
        try credential.put(arena, "refresh", .{ .string = try arena.dupe(u8, fields.refresh) });
        try credential.put(arena, "expires", .{ .integer = fields.expires });
        try credential.put(arena, "version", .{ .integer = @intCast(version) });
        if (fields.account_id) |value| try credential.put(arena, "account_id", .{ .string = try arena.dupe(u8, value) });
        if (fields.profile) |value| try credential.put(arena, "profile", .{ .string = try arena.dupe(u8, value) });
        if (fields.token_url) |value| try credential.put(arena, "token_url", .{ .string = try arena.dupe(u8, value) });
        if (fields.api_base) |value| try credential.put(arena, "api_base", .{ .string = try arena.dupe(u8, value) });
        try self.commitCredential(directory, provider, account, .{ .object = credential }, use);
    }

    /// Publish a network refresh only if the exact credential generation used
    /// to obtain it still owns this account. The reload and comparison happen
    /// under both locks, so logout and a replacement login always win.
    fn publishOAuthRefresh(self: *Store, provider: []const u8, account: []const u8, expected_refresh: []const u8, expected_profile: ?[]const u8, expected_version: u64, access_value: []const u8, refresh_value: []const u8, expires: i64, account_id: ?[]const u8, token_url: ?[]const u8, api_base: ?[]const u8) ![]const u8 {
        try credential_mutex.lock(self.io);
        defer credential_mutex.unlock(self.io);
        var directory = try self.prepareDirectory();
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);

        const current = (self.providerAccounts(provider) orelse return error.CredentialMissing).get(account) orelse return error.CredentialMissing;
        if (current != .object) return currentAccess(current);
        const credential = current.object;
        const current_refresh = optionalString(credential, "refresh") orelse return currentAccess(current);
        const current_profile = optionalString(credential, "profile");
        const current_version = credentialVersion(credential) catch return currentAccess(current);
        if (!std.mem.eql(u8, current_refresh, expected_refresh) or !optionalEql(current_profile, expected_profile) or current_version != expected_version)
            return currentAccess(current);

        if (expected_version == std.math.maxInt(i64)) return error.InvalidCredential;
        // A refresh never changes the active account, only its credential.
        try self.writeOAuth(directory, provider, account, .{
            .access = access_value,
            .refresh = refresh_value,
            .expires = expires,
            .account_id = account_id,
            .profile = expected_profile,
            .token_url = token_url,
            .api_base = api_base,
        }, expected_version + 1, false);
        return self.getAccount(provider, account) orelse return error.InvalidCredential;
    }

    /// Write one account's credential, optionally making it the active one,
    /// then publish. The caller owns both locks and has already reloaded.
    fn commitCredential(self: *Store, directory: std.Io.Dir, provider: []const u8, account: []const u8, credential: std.json.Value, use: bool) !void {
        const arena = self.parsed.arena.allocator();
        const entry = try self.ensureProvider(provider);
        const accounts = &entry.getPtr(accounts_field).?.object;
        try accounts.put(arena, try arena.dupe(u8, account), credential);
        if (use) try entry.put(arena, active_field, .{ .string = try arena.dupe(u8, account) });
        try self.save(directory);
    }

    fn ensureProvider(self: *Store, provider: []const u8) !*std.json.ObjectMap {
        const arena = self.parsed.arena.allocator();
        const root = &self.parsed.value.object;
        if (root.getPtr(provider)) |existing| {
            if (existing.* == .object) {
                if (existing.object.getPtr(accounts_field)) |accounts| {
                    if (accounts.* == .object) return &existing.object;
                }
            }
        }
        var entry: std.json.ObjectMap = .{};
        try entry.put(arena, accounts_field, .{ .object = .{} });
        try entry.put(arena, active_field, .{ .string = try arena.dupe(u8, default_account) });
        const name = try arena.dupe(u8, provider);
        try root.put(arena, name, .{ .object = entry });
        return &root.getPtr(name).?.object;
    }

    fn setActive(self: *Store, provider: []const u8, account: []const u8) !void {
        const arena = self.parsed.arena.allocator();
        const entry = try self.ensureProvider(provider);
        try entry.put(arena, active_field, .{ .string = try arena.dupe(u8, account) });
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
        try self.normalize();
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

/// Adopt the single-credential format by moving a provider's value into its
/// `default` account, and repair an entry that lost a valid active account.
fn normalizeProvider(arena: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    if (value == .object) {
        if (value.object.get(accounts_field)) |accounts| {
            if (accounts == .object) {
                var result = value;
                try repairActive(arena, &result.object);
                return result;
            }
        }
    }
    var credential: std.json.ObjectMap = .{};
    switch (value) {
        .string => |secret| {
            try credential.put(arena, "type", .{ .string = "api_key" });
            try credential.put(arena, "access", .{ .string = try arena.dupe(u8, secret) });
        },
        .object => |object| credential = object,
        else => {},
    }
    var accounts: std.json.ObjectMap = .{};
    try accounts.put(arena, try arena.dupe(u8, default_account), .{ .object = credential });
    var entry: std.json.ObjectMap = .{};
    try entry.put(arena, accounts_field, .{ .object = accounts });
    try entry.put(arena, active_field, .{ .string = try arena.dupe(u8, default_account) });
    return .{ .object = entry };
}

fn repairActive(arena: std.mem.Allocator, entry: *std.json.ObjectMap) !void {
    const accounts = &(entry.getPtr(accounts_field) orelse return).object;
    if (accounts.count() == 0) return;
    if (entry.get(active_field)) |active| {
        if (active == .string and accounts.get(active.string) != null) return;
    }
    var fallback: ?[]const u8 = null;
    var iterator = accounts.iterator();
    while (iterator.next()) |item| {
        const name = item.key_ptr.*;
        if (std.mem.eql(u8, name, default_account)) {
            fallback = name;
            break;
        }
        if (fallback == null or std.mem.order(u8, name, fallback.?) == .lt) fallback = name;
    }
    try entry.put(arena, active_field, .{ .string = try arena.dupe(u8, fallback.?) });
}

fn accountBefore(_: void, left: []u8, right: []u8) bool {
    const left_default = std.mem.eql(u8, left, default_account);
    if (left_default != std.mem.eql(u8, right, default_account)) return left_default;
    return std.mem.order(u8, left, right) == .lt;
}

fn secretOf(value: std.json.Value) ?[]const u8 {
    return switch (value) {
        .string => |secret| secret,
        .object => |credential| optionalString(credential, "access"),
        else => null,
    };
}
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
    if (isGenericProvider(id)) {
        const credential_store = store orelse return error.CredentialStoreUnavailable;
        const api_base = credential_store.getField(id, "api_base") orelse return error.CredentialProfileMissing;
        if (!validGenericApiBase(api_base) or !urlUnder(url, api_base)) return error.CredentialProfileMismatch;
        return;
    }
    const trusted: ?[]const u8 = if (std.mem.eql(u8, id, "openai"))
        "https://api.openai.com"
    else if (std.mem.eql(u8, id, "deepseek"))
        "https://api.deepseek.com"
    else if (std.mem.eql(u8, id, "groq"))
        "https://api.groq.com"
    else if (std.mem.eql(u8, id, "together"))
        "https://api.together.ai"
    else if (std.mem.eql(u8, id, "fireworks"))
        "https://api.fireworks.ai"
    else if (std.mem.eql(u8, id, "xai"))
        "https://api.x.ai"
    else if (std.mem.eql(u8, id, "mistral"))
        "https://api.mistral.ai"
    else if (std.mem.eql(u8, id, "cerebras"))
        "https://api.cerebras.ai"
    else if (std.mem.eql(u8, id, "deepinfra"))
        "https://api.deepinfra.com"
    else if (std.mem.eql(u8, id, "huggingface"))
        "https://router.huggingface.co"
    else if (std.mem.eql(u8, id, "nvidia"))
        "https://integrate.api.nvidia.com"
    else if (std.mem.eql(u8, id, "moonshot"))
        "https://api.moonshot.ai"
    else if (std.mem.eql(u8, id, "novita"))
        "https://api.novita.ai"
    else if (std.mem.eql(u8, id, "siliconflow"))
        "https://api.siliconflow.com"
    else if (std.mem.eql(u8, id, "venice"))
        "https://api.venice.ai"
    else if (std.mem.eql(u8, id, "anthropic"))
        "https://api.anthropic.com"
    else if (std.mem.eql(u8, id, "openrouter"))
        "https://openrouter.ai"
    else if (std.mem.eql(u8, id, "openai-codex"))
        "https://chatgpt.com/backend-api/codex"
    else if (std.mem.eql(u8, id, "brave"))
        "https://api.search.brave.com"
    else if (std.mem.eql(u8, id, "tavily"))
        "https://api.tavily.com"
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

fn isGenericProvider(provider: []const u8) bool {
    return std.mem.startsWith(u8, provider, "generic/") and provider.len > "generic/".len;
}

fn validGenericApiBase(value: []const u8) bool {
    if (!std.mem.startsWith(u8, value, "https://") or value.len <= "https://".len) return false;
    return std.mem.indexOfAny(u8, value["https://".len..], "?#\r\n\x00") == null and value[value.len - 1] != '/';
}

fn urlUnder(url: []const u8, base: []const u8) bool {
    return std.mem.startsWith(u8, url, base) and (url.len == base.len or url[base.len] == '/' or url[base.len] == '?' or url[base.len] == '#');
}

pub const Action = enum { login, logout, status, select };
pub const Strategy = enum { api_key, cli_handoff, device_oauth, loopback_pkce };

/// Provider-owned transport declaration validated against native trusted endpoints.
pub const Declaration = struct {
    provider: []const u8,
    strategy: Strategy,
    profile_id: ?[]const u8 = null,
    authorization_url: ?[]const u8 = null,
    token_url: ?[]const u8 = null,
    api_base: ?[]const u8 = null,
    provision_url: ?[]const u8 = null,

    pub fn parse(provider: []const u8, strategy_name: []const u8, profile_value: std.json.Value) !Declaration {
        const strategy = std.meta.stringToEnum(Strategy, strategy_name) orelse return error.UntrustedAuthDeclaration;
        var result: Declaration = .{ .provider = provider, .strategy = strategy };
        if (profile_value == .object) {
            const object = profile_value.object;
            result.profile_id = objectString(object, "id");
            result.authorization_url = objectString(object, "authorization_url");
            result.token_url = objectString(object, "token_url");
            result.api_base = objectString(object, "api_base");
            result.provision_url = objectString(object, "provision_url");
        } else if (profile_value != .null) return error.UntrustedAuthDeclaration;
        try result.validate();
        return result;
    }
    pub fn validate(self: Declaration) !void {
        if (apiKeyProvider(self.provider)) {
            if (self.strategy != .api_key or (self.provision_url != null and !validGenericApiBase(self.provision_url.?))) return error.UntrustedAuthDeclaration;
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
        } else if (isGenericProvider(self.provider)) {
            if (!validGenericApiBase(self.api_base orelse return error.UntrustedAuthDeclaration)) return error.UntrustedAuthDeclaration;
            if (self.strategy == .api_key) {
                if (self.provision_url != null and !validGenericApiBase(self.provision_url.?)) return error.UntrustedAuthDeclaration;
            } else if (self.strategy == .device_oauth) {
                if (self.profile_id == null or !validGenericApiBase(self.authorization_url orelse return error.UntrustedAuthDeclaration) or !validGenericApiBase(self.token_url orelse return error.UntrustedAuthDeclaration)) return error.UntrustedAuthDeclaration;
            } else return error.UntrustedAuthDeclaration;
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

fn apiKeyProvider(provider: []const u8) bool {
    return std.mem.eql(u8, provider, "openai") or std.mem.eql(u8, provider, "deepseek") or
        std.mem.eql(u8, provider, "anthropic") or std.mem.eql(u8, provider, "groq") or
        std.mem.eql(u8, provider, "together") or std.mem.eql(u8, provider, "fireworks") or
        std.mem.eql(u8, provider, "xai") or std.mem.eql(u8, provider, "mistral") or
        std.mem.eql(u8, provider, "cerebras") or std.mem.eql(u8, provider, "deepinfra") or
        std.mem.eql(u8, provider, "huggingface") or std.mem.eql(u8, provider, "nvidia") or
        std.mem.eql(u8, provider, "moonshot") or std.mem.eql(u8, provider, "novita") or
        std.mem.eql(u8, provider, "siliconflow") or std.mem.eql(u8, provider, "venice") or
        std.mem.eql(u8, provider, "brave") or std.mem.eql(u8, provider, "tavily");
}

pub const CommandResult = struct {
    logged_in: bool,
    subscription_type: ?[]u8 = null,
    /// The account this result refers to: the active one for status, the
    /// touched one for login, logout, and select.
    account: ?[]u8 = null,
    /// Human-readable outcome of this action. The caller owns it.
    message: ?[]u8 = null,

    pub fn deinit(self: CommandResult, allocator: std.mem.Allocator) void {
        if (self.subscription_type) |value| allocator.free(value);
        if (self.account) |value| allocator.free(value);
        if (self.message) |value| allocator.free(value);
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

pub fn commandTerminal(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: Action, provider: []const u8, account: ?[]const u8) !CommandResult {
    const declaration: Declaration = if (apiKeyProvider(provider)) .{ .provider = provider, .strategy = .api_key, .provision_url = provisioningUrl(provider) } else if (std.mem.eql(u8, provider, "claude")) .{ .provider = provider, .strategy = .cli_handoff } else if (std.mem.eql(u8, provider, "openrouter")) .{ .provider = provider, .strategy = .loopback_pkce, .profile_id = "default" } else if (std.mem.eql(u8, provider, "openai-codex")) .{ .provider = provider, .strategy = .device_oauth, .profile_id = "default", .authorization_url = "https://auth.openai.com/api/accounts/deviceauth/usercode", .token_url = "https://auth.openai.com/oauth/token" } else if (std.mem.eql(u8, provider, "kimi-coding")) .{ .provider = provider, .strategy = .device_oauth, .profile_id = "global", .authorization_url = "https://auth.kimi.ai/api/oauth/device_authorization", .token_url = "https://auth.kimi.ai/api/oauth/token", .api_base = "https://api.kimi.ai/coding/v1" } else return error.UnknownProvider;
    var context = TerminalInteraction{ .allocator = allocator, .io = io };
    defer if (context.owned_input) |value| {
        std.crypto.secureZero(u8, value);
        allocator.free(value);
    };
    return command(allocator, io, environ, action, declaration, .{ .context = &context, .emitFn = TerminalInteraction.emit, .inputFn = TerminalInteraction.input }, account);
}

fn provisioningUrl(provider: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, provider, "deepseek")) return "https://platform.deepseek.com/api_keys";
    if (std.mem.eql(u8, provider, "groq")) return "https://console.groq.com/keys";
    if (std.mem.eql(u8, provider, "together")) return "https://api.together.ai/settings/api-keys";
    if (std.mem.eql(u8, provider, "xai")) return "https://console.x.ai";
    if (std.mem.eql(u8, provider, "mistral")) return "https://console.mistral.ai/api-keys";
    if (std.mem.eql(u8, provider, "cerebras")) return "https://cloud.cerebras.ai";
    if (std.mem.eql(u8, provider, "deepinfra")) return "https://deepinfra.com/dash/api_keys";
    if (std.mem.eql(u8, provider, "huggingface")) return "https://huggingface.co/settings/tokens";
    if (std.mem.eql(u8, provider, "nvidia")) return "https://build.nvidia.com";
    if (std.mem.eql(u8, provider, "novita")) return "https://novita.ai/settings/key-management";
    if (std.mem.eql(u8, provider, "venice")) return "https://venice.ai/settings/api";
    if (std.mem.eql(u8, provider, "brave")) return "https://api-dashboard.search.brave.com/app/keys";
    if (std.mem.eql(u8, provider, "tavily")) return "https://app.tavily.com/home";
    return null;
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

pub fn command(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, action: Action, declaration: Declaration, interaction: Interaction, account: ?[]const u8) !CommandResult {
    try declaration.validate();
    const provider = declaration.provider;
    if (account) |name| if (!validAccountName(name)) return error.InvalidAccount;
    if (std.mem.eql(u8, provider, "claude")) {
        // Claude Code owns its single account: Misa stores none for it, so a
        // named account cannot refer to anything and there is nothing to select.
        if (account != null or action == .select) return error.UnknownAccount;
        return commandCli(allocator, io, environ, action, "claude");
    }
    if (!managedProvider(provider)) return error.UnknownProvider;
    var store = try Store.init(allocator, io, environ);
    defer store.deinit();
    switch (action) {
        .status => return statusResult(allocator, &store, provider, account),
        .select => return selectResult(allocator, &store, provider, account orelse return error.MissingAccount),
        .logout => {
            var result: CommandResult = .{ .logged_in = false };
            errdefer result.deinit(allocator);
            if (account) |name| {
                if (!try store.removeAccount(provider, name)) return error.UnknownAccount;
                result.account = try allocator.dupe(u8, name);
                result.message = try logoutMessage(allocator, name);
            } else {
                _ = try store.remove(provider);
                result.message = try logoutMessage(allocator, null);
            }
            result.logged_in = store.contains(provider);
            return result;
        },
        .login => {},
    }
    // Without a named account, login replaces the credential in use.
    const target = account orelse store.activeAccount(provider) orelse default_account;
    if (std.mem.eql(u8, provider, "openai-codex") or std.mem.eql(u8, provider, "kimi-coding") or (isGenericProvider(provider) and declaration.strategy == .device_oauth)) {
        const credential = if (std.mem.eql(u8, provider, "openai-codex"))
            try oauth.loginOpenAI(allocator, io, interaction)
        else if (isGenericProvider(provider))
            try oauth.loginDevice(allocator, io, declaration.authorization_url.?, declaration.token_url.?, declaration.profile_id.?, interaction)
        else
            try oauth.loginKimi(allocator, io, declaration.authorization_url.?, declaration.token_url.?, interaction);
        defer credential.deinit(allocator);
        try store.putOAuthMetadataAccount(provider, target, credential.access, credential.refresh, credential.expires, credential.account_id, declaration.profile_id, declaration.token_url, declaration.api_base);
    } else if (std.mem.eql(u8, provider, "openrouter")) {
        const authorization = try oauth.startOpenRouter(allocator, io);
        defer authorization.deinit(allocator);
        const input = try oauth.authorizeOpenRouter(allocator, io, authorization, interaction);
        defer allocator.free(input);
        const credential = try oauth.finishOpenRouter(allocator, io, authorization.verifier, input);
        defer credential.deinit(allocator);
        try store.putOAuthMetadataAccount(provider, target, credential.access, credential.refresh, credential.expires, credential.account_id, null, null, null);
    } else {
        if (interaction.protected_input) {
            try interaction.emit(.{ .correlation = "api-key", .kind = "modal", .title = "API key", .message = if (declaration.provision_url == null) "Enter your API key. Characters are hidden." else "Open the key page, create a key, then paste it here. Characters are hidden.", .url = declaration.provision_url, .input = true, .protected = true });
            const secret = try interaction.input("api-key");
            if (secret.len == 0) return error.EmptyCredential;
            if (isGenericProvider(provider)) try store.putAccountApiKeyMetadata(provider, target, secret, declaration.api_base.?) else try store.putAccount(provider, target, secret);
        } else {
            if (declaration.provision_url) |url|
                try interaction.emit(.{ .correlation = "api-key", .kind = "progress", .title = "Create an API key", .message = "Open this page to create an API key, then paste it here.", .url = url });
            const secret = try readSecret(allocator, io, "API key: ");
            defer {
                std.crypto.secureZero(u8, secret);
                allocator.free(secret);
            }
            if (isGenericProvider(provider)) try store.putAccountApiKeyMetadata(provider, target, secret, declaration.api_base.?) else try store.putAccount(provider, target, secret);
        }
    }
    if (!interaction.protected_input) {
        if (std.mem.eql(u8, target, default_account))
            std.debug.print("misa: saved {s} credential to {s}\n", .{ provider, store.path })
        else
            std.debug.print("misa: saved {s} credential for account {s} to {s}\n", .{ provider, target, store.path });
    }
    var result: CommandResult = .{ .logged_in = true };
    errdefer result.deinit(allocator);
    result.account = try allocator.dupe(u8, target);
    result.message = try loginMessage(allocator, target);
    return result;
}

/// Describe every stored account of a provider, or one named account. The
/// caller owns the returned message and account.
pub fn statusResult(allocator: std.mem.Allocator, store: *const Store, provider: []const u8, requested: ?[]const u8) !CommandResult {
    var result: CommandResult = .{ .logged_in = false };
    errdefer result.deinit(allocator);
    if (store.activeAccount(provider)) |name| result.account = try allocator.dupe(u8, name);
    if (requested) |name| {
        result.logged_in = store.containsAccount(provider, name);
        result.message = try statusMessage(allocator, name, result.logged_in);
        return result;
    }
    const names = try store.accountNames(allocator, provider);
    defer {
        for (names) |name| allocator.free(name);
        allocator.free(names);
    }
    result.logged_in = result.account != null;
    result.message = try accountsMessage(allocator, names, result.account);
    return result;
}

/// Switch the account a provider's requests use, without re-authenticating.
pub fn selectResult(allocator: std.mem.Allocator, store: *Store, provider: []const u8, account: []const u8) !CommandResult {
    if (!try store.select(provider, account)) return error.UnknownAccount;
    var result: CommandResult = .{ .logged_in = true };
    errdefer result.deinit(allocator);
    result.account = try allocator.dupe(u8, account);
    result.message = if (std.mem.eql(u8, account, default_account))
        try allocator.dupe(u8, "using the default account")
    else
        try std.fmt.allocPrint(allocator, "using account {s}", .{account});
    return result;
}

fn statusMessage(allocator: std.mem.Allocator, account: []const u8, logged_in: bool) ![]u8 {
    const state = if (logged_in) "logged in" else "logged out";
    if (std.mem.eql(u8, account, default_account)) return allocator.dupe(u8, state);
    return std.fmt.allocPrint(allocator, "{s}: {s}", .{ account, state });
}

fn accountsMessage(allocator: std.mem.Allocator, names: []const []u8, active: ?[]const u8) ![]u8 {
    if (names.len == 0) return allocator.dupe(u8, "logged out");
    // A lone account needs no marker: there is nothing to choose between.
    if (names.len == 1) return statusMessage(allocator, names[0], true);
    var text: std.ArrayList(u8) = .empty;
    errdefer text.deinit(allocator);
    for (names, 0..) |name, index| {
        if (index != 0) try text.append(allocator, '\n');
        try text.appendSlice(allocator, name);
        try text.appendSlice(allocator, ": logged in");
        if (active != null and std.mem.eql(u8, name, active.?)) try text.appendSlice(allocator, " (active)");
    }
    return text.toOwnedSlice(allocator);
}

pub fn loginMessage(allocator: std.mem.Allocator, account: []const u8) ![]u8 {
    if (std.mem.eql(u8, account, default_account)) return allocator.dupe(u8, "logged in");
    return std.fmt.allocPrint(allocator, "logged in as {s}", .{account});
}

pub fn logoutMessage(allocator: std.mem.Allocator, account: ?[]const u8) ![]u8 {
    if (account == null or std.mem.eql(u8, account.?, default_account)) return allocator.dupe(u8, "logged out");
    return std.fmt.allocPrint(allocator, "logged out as {s}", .{account.?});
}

fn managedProvider(provider: []const u8) bool {
    return apiKeyProvider(provider) or std.mem.eql(u8, provider, "openai-codex") or
        std.mem.eql(u8, provider, "openrouter") or
        std.mem.eql(u8, provider, "kimi-coding") or isGenericProvider(provider);
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
    // Claude reports its own single account, so the wording never names one.
    var result: CommandResult = .{ .logged_in = logged_in, .subscription_type = subscription };
    errdefer result.deinit(allocator);
    result.message = try statusMessage(allocator, default_account, logged_in);
    return result;
}

test "a CLI-owned provider rejects accounts and names none in status" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    const declaration: Declaration = .{ .provider = "claude", .strategy = .cli_handoff };
    const unused: Interaction = .{ .context = undefined, .emitFn = undefined, .inputFn = undefined };
    for ([_]Action{ .select, .status, .logout }) |action| {
        try std.testing.expectError(error.UnknownAccount, command(std.testing.allocator, std.testing.io, &environ, action, declaration, unused, "work"));
    }
    try std.testing.expectError(error.UnknownAccount, command(std.testing.allocator, std.testing.io, &environ, .select, declaration, unused, default_account));
    try std.testing.expectError(error.UnknownAccount, command(std.testing.allocator, std.testing.io, &environ, .select, declaration, unused, null));
    // Status wording for a CLI-owned provider never names an account.
    const message = try statusMessage(std.testing.allocator, default_account, true);
    defer std.testing.allocator.free(message);
    try std.testing.expectEqualStrings("logged in", message);
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

test "generic API-key providers require an HTTPS base URL" {
    const valid: Declaration = .{ .provider = "generic/local", .strategy = .api_key, .api_base = "https://llm.example/v1" };
    try valid.validate();
    const deepseek: Declaration = .{ .provider = "deepseek", .strategy = .api_key };
    try deepseek.validate();
    var invalid = valid;
    invalid.api_base = "http://llm.example/v1";
    try std.testing.expectError(error.UntrustedAuthDeclaration, invalid.validate());
}

test "dedicated search credentials are API-key providers" {
    const brave: Declaration = .{ .provider = "brave", .strategy = .api_key, .provision_url = "https://api-dashboard.search.brave.com/app/keys" };
    try brave.validate();
    const tavily: Declaration = .{ .provider = "tavily", .strategy = .api_key, .provision_url = "https://app.tavily.com/home" };
    try tavily.validate();
    try std.testing.expect(apiKeyProvider("brave"));
    try std.testing.expect(apiKeyProvider("tavily"));
    try std.testing.expect(managedProvider("brave"));
    try std.testing.expect(managedProvider("tavily"));
    const drifted: Declaration = .{ .provider = "brave", .strategy = .device_oauth };
    try std.testing.expectError(error.UntrustedAuthDeclaration, drifted.validate());
}

test "generic device OAuth requires trusted HTTPS endpoints and a client ID" {
    const valid: Declaration = .{ .provider = "generic/acme", .strategy = .device_oauth, .profile_id = "acme-cli", .authorization_url = "https://login.example/device", .token_url = "https://login.example/token", .api_base = "https://api.example/v1" };
    try valid.validate();
    var invalid = valid;
    invalid.profile_id = null;
    try std.testing.expectError(error.UntrustedAuthDeclaration, invalid.validate());
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
    try std.testing.expectError(error.CredentialMissing, refreshing.publishOAuthRefresh("openai-codex", "default", "old-refresh", "default", 1, "stale-access", "stale-refresh", 100, null, "https://auth.example/token", null));
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
    const access_value = try refreshing.publishOAuthRefresh("openai-codex", "default", "same-refresh", "default", 1, "stale-access", "stale-refresh", 200, null, null, null);
    try std.testing.expectEqualStrings("new-login-access", access_value);
    try std.testing.expectEqualStrings("same-refresh", refreshing.getField("openai-codex", "refresh").?);
}

test "standard credential origins require native trust or explicit user grant" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try validateCredentialOrigin("openai", "https://api.openai.com/v1/responses", null, &environ);
    try validateCredentialOrigin("brave", "https://api.search.brave.com/res/v1/web/search", null, &environ);
    try validateCredentialOrigin("tavily", "https://api.tavily.com/search", null, &environ);
    try std.testing.expectError(error.CredentialOriginDenied, validateCredentialOrigin("brave", "https://evil.example/v1", null, &environ));
    try std.testing.expectError(error.CredentialOriginDenied, validateCredentialOrigin("tavily", "https://api.tavily.com.evil/search", null, &environ));
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

test "a provider keeps one credential per account and uses the active one" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/auth.json", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_AUTH_FILE", path);
    var store = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer store.deinit();
    try store.putAccount("openai", default_account, "personal");
    try store.putAccount("openai", "work", "corporate");
    // The most recent login is the one this provider uses.
    try std.testing.expectEqualStrings("work", store.activeAccount("openai").?);
    try std.testing.expectEqualStrings("corporate", store.get("openai").?);
    try std.testing.expectEqualStrings("personal", store.getAccount("openai", default_account).?);
    const names = try store.accountNames(std.testing.allocator, "openai");
    defer {
        for (names) |name| std.testing.allocator.free(name);
        std.testing.allocator.free(names);
    }
    try std.testing.expectEqual(@as(usize, 2), names.len);
    try std.testing.expectEqualStrings(default_account, names[0]);
    try std.testing.expectEqualStrings("work", names[1]);
    try std.testing.expect(try store.select("openai", default_account));
    try std.testing.expectEqualStrings("personal", store.get("openai").?);
    try std.testing.expect(!try store.select("openai", "absent"));
    // Removing the account in use falls back to one that remains.
    try std.testing.expect(try store.removeAccount("openai", default_account));
    try std.testing.expectEqualStrings("work", store.activeAccount("openai").?);
    try std.testing.expect(try store.removeAccount("openai", "work"));
    try std.testing.expect(!store.contains("openai"));
}

test "a credential written before accounts existed becomes the default account" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    try temporary.dir.writeFile(std.testing.io, .{ .sub_path = "auth.json", .data = "{\"openai\":\"legacy\",\"other\":{\"access\":\"value\"}}" });
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/auth.json", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_AUTH_FILE", path);
    var store = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer store.deinit();
    try std.testing.expectEqualStrings(default_account, store.activeAccount("openai").?);
    try std.testing.expectEqualStrings("legacy", store.get("openai").?);
    try std.testing.expectEqualStrings("value", store.get("other").?);
    const names = try store.accountNames(std.testing.allocator, "openai");
    defer {
        for (names) |name| std.testing.allocator.free(name);
        std.testing.allocator.free(names);
    }
    try std.testing.expectEqual(@as(usize, 1), names.len);
    // A second login rewrites the document in the account format.
    try store.putAccount("openai", "work", "second");
    var reopened = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer reopened.deinit();
    try std.testing.expectEqualStrings("work", reopened.activeAccount("openai").?);
    try std.testing.expectEqualStrings("legacy", reopened.getAccount("openai", default_account).?);
}

test "status names each stored account and marks the active one" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/auth.json", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_AUTH_FILE", path);
    var store = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer store.deinit();
    var unauthenticated = try statusResult(std.testing.allocator, &store, "openai", null);
    defer unauthenticated.deinit(std.testing.allocator);
    try std.testing.expect(!unauthenticated.logged_in);
    try std.testing.expectEqualStrings("logged out", unauthenticated.message.?);

    // A lone default account keeps the plain wording.
    try store.putAccount("openai", default_account, "personal");
    var only_default = try statusResult(std.testing.allocator, &store, "openai", null);
    defer only_default.deinit(std.testing.allocator);
    try std.testing.expect(only_default.logged_in);
    try std.testing.expectEqualStrings("logged in", only_default.message.?);

    try store.putAccount("openai", "work", "corporate");
    var both = try statusResult(std.testing.allocator, &store, "openai", null);
    defer both.deinit(std.testing.allocator);
    try std.testing.expectEqualStrings("default: logged in\nwork: logged in (active)", both.message.?);
    var named = try statusResult(std.testing.allocator, &store, "openai", "work");
    defer named.deinit(std.testing.allocator);
    try std.testing.expectEqualStrings("work: logged in", named.message.?);
    var absent = try statusResult(std.testing.allocator, &store, "openai", "absent");
    defer absent.deinit(std.testing.allocator);
    try std.testing.expect(!absent.logged_in);
    try std.testing.expectEqualStrings("absent: logged out", absent.message.?);

    var selected = try selectResult(std.testing.allocator, &store, "openai", default_account);
    defer selected.deinit(std.testing.allocator);
    try std.testing.expectEqualStrings("using the default account", selected.message.?);
    try std.testing.expectError(error.UnknownAccount, selectResult(std.testing.allocator, &store, "openai", "absent"));
    // A lone remaining account states its name without a marker.
    try std.testing.expect(try store.removeAccount("openai", default_account));
    var lone = try statusResult(std.testing.allocator, &store, "openai", null);
    defer lone.deinit(std.testing.allocator);
    try std.testing.expectEqualStrings("work", lone.account.?);
    try std.testing.expectEqualStrings("work: logged in", lone.message.?);
}

test "login and logout report the account they touched" {
    const plain = try loginMessage(std.testing.allocator, default_account);
    defer std.testing.allocator.free(plain);
    try std.testing.expectEqualStrings("logged in", plain);
    const named = try loginMessage(std.testing.allocator, "work");
    defer std.testing.allocator.free(named);
    try std.testing.expectEqualStrings("logged in as work", named);
    const all = try logoutMessage(std.testing.allocator, null);
    defer std.testing.allocator.free(all);
    try std.testing.expectEqualStrings("logged out", all);
    const one = try logoutMessage(std.testing.allocator, "work");
    defer std.testing.allocator.free(one);
    try std.testing.expectEqualStrings("logged out as work", one);
    try std.testing.expect(validAccountName("work-2.1"));
    try std.testing.expect(!validAccountName(""));
    try std.testing.expect(!validAccountName("two words"));
}
