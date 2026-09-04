//! Bounded workspace file effects used by tool extensions.
const std = @import("std");

const max_bytes = 1024 * 1024;

pub const Spec = union(enum) {
    read: struct { path: []const u8, completion: []const u8, id: []const u8 },
    list: struct { path: []const u8, completion: []const u8, id: []const u8 },
    write: struct { path: []const u8, content: []const u8, completion: []const u8, id: []const u8 },
    edit: struct { path: []const u8, old: []const u8, new: []const u8, completion: []const u8, id: []const u8 },

    pub fn parse(kind: []const u8, object: std.json.ObjectMap) !Spec {
        const path = nonEmptyString(object, "path") orelse return error.InvalidEffect;
        const completion_name = nonEmptyString(object, "completion") orelse return error.InvalidEffect;
        const id = nonEmptyString(object, "id") orelse return error.InvalidEffect;
        if (std.mem.eql(u8, kind, "file/read")) return .{ .read = .{ .path = path, .completion = completion_name, .id = id } };
        if (std.mem.eql(u8, kind, "file/list")) return .{ .list = .{ .path = path, .completion = completion_name, .id = id } };
        const content = string(object, "content") orelse return error.InvalidEffect;
        if (content.len > max_bytes) return error.EffectTooLarge;
        if (std.mem.eql(u8, kind, "file/write")) return .{ .write = .{ .path = path, .content = content, .completion = completion_name, .id = id } };
        if (!std.mem.eql(u8, kind, "file/edit")) return error.UnknownNativeEffect;
        const replacement = string(object, "replacement") orelse return error.InvalidEffect;
        if (content.len == 0 or replacement.len > max_bytes) return error.InvalidEffect;
        return .{ .edit = .{ .path = path, .old = content, .new = replacement, .completion = completion_name, .id = id } };
    }

    pub fn completion(self: Spec) []const u8 {
        return switch (self) {
            inline else => |value| value.completion,
        };
    }

    pub fn requestId(self: Spec) []const u8 {
        return switch (self) {
            inline else => |value| value.id,
        };
    }
};

pub fn run(allocator: std.mem.Allocator, io: std.Io, spec: Spec) ![]u8 {
    return switch (spec) {
        .read => |read| std.Io.Dir.cwd().readFileAlloc(io, read.path, allocator, .limited(max_bytes)),
        .list => |list| listDirectory(allocator, io, list.path),
        .write => |write| blk: {
            const permissions = if (std.Io.Dir.cwd().statFile(io, write.path, .{})) |stat| stat.permissions else |_| std.Io.File.Permissions.default_file;
            var atomic = try std.Io.Dir.cwd().createFileAtomic(io, write.path, .{ .permissions = permissions, .make_path = true, .replace = true });
            defer atomic.deinit(io);
            try atomic.file.writeStreamingAll(io, write.content);
            try atomic.replace(io);
            break :blk try std.fmt.allocPrint(allocator, "wrote {d} bytes to {s}", .{ write.content.len, write.path });
        },
        .edit => |edit| blk: {
            const source = try std.Io.Dir.cwd().readFileAlloc(io, edit.path, allocator, .limited(max_bytes));
            defer allocator.free(source);
            const first = std.mem.indexOf(u8, source, edit.old) orelse return error.PatternNotFound;
            if (std.mem.indexOfPos(u8, source, first + edit.old.len, edit.old) != null) return error.PatternNotUnique;
            const output = try std.mem.replaceOwned(u8, allocator, source, edit.old, edit.new);
            defer allocator.free(output);
            const permissions = (try std.Io.Dir.cwd().statFile(io, edit.path, .{})).permissions;
            var atomic = try std.Io.Dir.cwd().createFileAtomic(io, edit.path, .{ .permissions = permissions, .replace = true });
            defer atomic.deinit(io);
            try atomic.file.writeStreamingAll(io, output);
            try atomic.replace(io);
            break :blk try std.fmt.allocPrint(allocator, "edited {s}", .{edit.path});
        },
    };
}

fn listDirectory(allocator: std.mem.Allocator, io: std.Io, path: []const u8) ![]u8 {
    var directory = try std.Io.Dir.cwd().openDir(io, path, .{ .iterate = true });
    defer directory.close(io);
    var iterator = directory.iterate();
    var entries: std.ArrayList([]u8) = .empty;
    defer {
        for (entries.items) |entry| allocator.free(entry);
        entries.deinit(allocator);
    }
    while (try iterator.next(io)) |entry| {
        if (entries.items.len >= 10_000) return error.TooManyEntries;
        const suffix: []const u8 = if (entry.kind == .directory) "/" else "";
        try entries.append(allocator, try std.fmt.allocPrint(allocator, "{s}{s}", .{ entry.name, suffix }));
    }
    std.mem.sort([]u8, entries.items, {}, struct {
        fn lessThan(_: void, left: []u8, right: []u8) bool {
            return std.mem.lessThan(u8, left, right);
        }
    }.lessThan);
    var output: std.ArrayList(u8) = .empty;
    errdefer output.deinit(allocator);
    for (entries.items) |entry| {
        if (output.items.len + entry.len + 1 > max_bytes) return error.StreamTooLong;
        try output.appendSlice(allocator, entry);
        try output.append(allocator, '\n');
    }
    return output.toOwnedSlice(allocator);
}

fn string(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    return switch (object.get(name) orelse return null) {
        .string => |value| value,
        else => null,
    };
}

fn nonEmptyString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = string(object, name) orelse return null;
    return if (value.len > 0 and std.mem.indexOfScalar(u8, value, 0) == null) value else null;
}
