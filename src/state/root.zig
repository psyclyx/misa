//! Bounded, namespaced application state persisted as one atomic JSON document.
const std = @import("std");

pub const max_file_size: usize = 1024 * 1024;
pub const max_nesting_depth: usize = 64;
pub const max_namespaces: usize = 64;
pub const max_namespace_len: usize = 128;
const empty_document = "{\"version\":1,\"namespaces\":{}}";
const lock_name = ".misa-state.lock";

pub const Store = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    path: []u8,
    parsed: std.json.Parsed(std.json.Value),
    protect_parent: bool,

    pub fn init(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) !Store {
        const path = try statePath(allocator, environ);
        errdefer allocator.free(path);
        const protect_parent = environ.get("MISA_STATE_FILE") == null;
        var directory = try prepareDirectory(io, path, protect_parent);
        defer directory.close(io);
        var lock = try lockDirectoryFile(io, directory);
        defer lock.close(io);
        const source = try readDocument(allocator, io, directory, std.fs.path.basename(path));
        defer allocator.free(source);
        var parsed = std.json.parseFromSlice(std.json.Value, allocator, source, .{ .allocate = .alloc_always }) catch return error.InvalidState;
        errdefer parsed.deinit();
        try validateDocument(parsed.value);
        return .{ .allocator = allocator, .io = io, .path = path, .parsed = parsed, .protect_parent = protect_parent };
    }

    pub fn deinit(self: *Store) void {
        self.parsed.deinit();
        self.allocator.free(self.path);
    }

    /// Reload while holding the interprocess lock so long-lived processes do
    /// not return a stale document after another Misa instance publishes.
    pub fn load(self: *Store, namespace: []const u8) !?std.json.Value {
        try validateNamespace(namespace);
        var directory = try prepareDirectory(self.io, self.path, self.protect_parent);
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);
        return self.parsed.value.object.get("namespaces").?.object.get(namespace);
    }

    /// Lock, reload, merge one namespace, atomically publish, then fsync the
    /// parent directory. Reload-under-lock prevents interprocess lost updates.
    pub fn save(self: *Store, namespace: []const u8, value: std.json.Value) !void {
        try validateNamespace(namespace);
        var nodes: usize = 0;
        try validateValue(value, 0, &nodes);

        var directory = try prepareDirectory(self.io, self.path, self.protect_parent);
        defer directory.close(self.io);
        var lock = try lockDirectoryFile(self.io, directory);
        defer lock.close(self.io);
        try self.reload(directory);

        const namespaces = &self.parsed.value.object.getPtr("namespaces").?.object;
        const existing = namespaces.get(namespace);
        if (existing == null and namespaces.count() >= max_namespaces) return error.StateTooLarge;
        const owned_value = try cloneJson(self.parsed.arena.allocator(), value);
        if (existing != null) {
            namespaces.getPtr(namespace).?.* = owned_value;
        } else {
            const key = try self.parsed.arena.allocator().dupe(u8, namespace);
            try namespaces.put(self.parsed.arena.allocator(), key, owned_value);
        }

        const document = try std.json.Stringify.valueAlloc(self.allocator, self.parsed.value, .{ .whitespace = .indent_2 });
        defer self.allocator.free(document);
        if (document.len > max_file_size) return error.StateTooLarge;

        var atomic = try directory.createFileAtomic(self.io, std.fs.path.basename(self.path), .{
            .permissions = @enumFromInt(0o600),
            .replace = true,
        });
        defer atomic.deinit(self.io);
        try atomic.file.writeStreamingAll(self.io, document);
        try atomic.file.sync(self.io);
        try atomic.replace(self.io);
        // rename durability requires syncing the containing directory.
        const directory_file: std.Io.File = .{ .handle = directory.handle, .flags = .{ .nonblocking = false } };
        try directory_file.sync(self.io);
    }

    fn reload(self: *Store, directory: std.Io.Dir) !void {
        const source = try readDocument(self.allocator, self.io, directory, std.fs.path.basename(self.path));
        defer self.allocator.free(source);
        var replacement = std.json.parseFromSlice(std.json.Value, self.allocator, source, .{ .allocate = .alloc_always }) catch return error.InvalidState;
        errdefer replacement.deinit();
        try validateDocument(replacement.value);
        self.parsed.deinit();
        self.parsed = replacement;
    }
};

fn prepareDirectory(io: std.Io, path: []const u8, protect: bool) !std.Io.Dir {
    const directory_path = std.fs.path.dirname(path) orelse ".";
    try std.Io.Dir.cwd().createDirPath(io, directory_path);
    var directory = try std.Io.Dir.cwd().openDir(io, directory_path, .{ .iterate = true, .follow_symlinks = false });
    errdefer directory.close(io);
    if (protect) try directory.setPermissions(io, @enumFromInt(0o700));
    return directory;
}

/// The lock is a sibling opened without following symlinks. Exclusive create
/// closes the not-found race; if another process wins, retry the hardened open.
fn lockDirectoryFile(io: std.Io, directory: std.Io.Dir) !std.Io.File {
    while (true) {
        return directory.openFile(io, lock_name, .{
            .mode = .read_write,
            .allow_directory = false,
            .follow_symlinks = false,
            .resolve_beneath = true,
            .lock = .exclusive,
        }) catch |err| switch (err) {
            error.FileNotFound => directory.createFile(io, lock_name, .{
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

fn readDocument(allocator: std.mem.Allocator, io: std.Io, directory: std.Io.Dir, basename: []const u8) ![]u8 {
    var file = directory.openFile(io, basename, .{
        .allow_directory = false,
        .follow_symlinks = false,
        .resolve_beneath = true,
    }) catch |err| switch (err) {
        error.FileNotFound => return allocator.dupe(u8, empty_document),
        else => return err,
    };
    defer file.close(io);
    var reader = file.reader(io, &.{});
    return reader.interface.allocRemaining(allocator, .limited(max_file_size)) catch |err| switch (err) {
        error.ReadFailed => return reader.err.?,
        else => return err,
    };
}

pub fn statePath(allocator: std.mem.Allocator, environ: *const std.process.Environ.Map) ![]u8 {
    if (environ.get("MISA_STATE_FILE")) |path| {
        if (path.len == 0 or std.mem.indexOfScalar(u8, path, 0) != null) return error.InvalidStatePath;
        return allocator.dupe(u8, path);
    }
    if (environ.get("XDG_STATE_HOME")) |root| return std.fs.path.join(allocator, &.{ root, "misa", "state" });
    if (environ.get("HOME")) |home| return std.fs.path.join(allocator, &.{ home, ".local", "state", "misa", "state" });
    return error.StatePathUnavailable;
}

fn validateDocument(value: std.json.Value) !void {
    const object = switch (value) {
        .object => |item| item,
        else => return error.InvalidState,
    };
    if (object.count() != 2) return error.InvalidState;
    const version = object.get("version") orelse return error.InvalidState;
    if (version != .integer or version.integer != 1) return error.InvalidState;
    const namespaces = switch (object.get("namespaces") orelse return error.InvalidState) {
        .object => |item| item,
        else => return error.InvalidState,
    };
    if (namespaces.count() > max_namespaces) return error.InvalidState;
    var iterator = namespaces.iterator();
    while (iterator.next()) |entry| {
        validateNamespace(entry.key_ptr.*) catch return error.InvalidState;
        var nodes: usize = 0;
        validateValue(entry.value_ptr.*, 0, &nodes) catch return error.InvalidState;
    }
}

pub fn validateNamespace(namespace: []const u8) !void {
    if (namespace.len == 0 or namespace.len > max_namespace_len) return error.InvalidNamespace;
    for (namespace) |byte| if (!(std.ascii.isAlphanumeric(byte) or byte == '.' or byte == '_' or byte == '-')) return error.InvalidNamespace;
}

fn validateValue(value: std.json.Value, depth: usize, nodes: *usize) !void {
    if (depth > max_nesting_depth) return error.StateTooDeep;
    nodes.* += 1;
    if (nodes.* > 100_000) return error.StateTooLarge;
    switch (value) {
        .array => |array| for (array.items) |item| try validateValue(item, depth + 1, nodes),
        .object => |object| {
            var iterator = object.iterator();
            while (iterator.next()) |entry| {
                if (entry.key_ptr.*.len > 4096) return error.InvalidStateValue;
                try validateValue(entry.value_ptr.*, depth + 1, nodes);
            }
        },
        .string, .number_string => |string| if (string.len > max_file_size) return error.InvalidStateValue,
        .float => |number| if (!std.math.isFinite(number)) return error.InvalidStateValue,
        else => {},
    }
}

fn cloneJson(allocator: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    const encoded = try std.json.Stringify.valueAlloc(allocator, value, .{});
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, encoded, .{ .allocate = .alloc_always });
    return parsed.value;
}

test "state paths follow override and XDG precedence" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("HOME", "/home/test");
    var path = try statePath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/home/test/.local/state/misa/state", path);
    std.testing.allocator.free(path);
    try environ.put("XDG_STATE_HOME", "/state");
    path = try statePath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/state/misa/state", path);
    std.testing.allocator.free(path);
    try environ.put("MISA_STATE_FILE", "/tmp/misa-state");
    path = try statePath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/tmp/misa-state", path);
    std.testing.allocator.free(path);
}

test "stores merge namespaces without lost updates" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/state", .{temporary.sub_path});
    defer std.testing.allocator.free(path);
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_STATE_FILE", path);

    var first = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer first.deinit();
    var second = try Store.init(std.testing.allocator, std.testing.io, &environ);
    defer second.deinit();
    var one = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"value\":1}", .{});
    defer one.deinit();
    var two = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"value\":2}", .{});
    defer two.deinit();
    try first.save("first", one.value);
    try second.save("second", two.value);
    try std.testing.expectEqual(@as(i64, 1), (try second.load("first")).?.object.get("value").?.integer);
    try std.testing.expectEqual(@as(i64, 2), (try first.load("second")).?.object.get("value").?.integer);
}

test "document schema and namespace are validated" {
    var bad = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"version\":2,\"namespaces\":{}}", .{});
    defer bad.deinit();
    try std.testing.expectError(error.InvalidState, validateDocument(bad.value));
    try std.testing.expectError(error.InvalidNamespace, validateNamespace("../escape"));
}
