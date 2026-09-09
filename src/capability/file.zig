//! Bounded workspace file effects used by tool extensions.
const std = @import("std");

const max_bytes = 1024 * 1024;
const max_read_lines = 2000;

pub const Spec = union(enum) {
    read: struct { path: []const u8, completion: []const u8, id: []const u8, anchored: bool = false, start_line: usize = 1, max_lines: usize = max_read_lines },
    list: struct { path: []const u8, completion: []const u8, id: []const u8 },
    write: struct { path: []const u8, content: []const u8, completion: []const u8, id: []const u8 },
    edit: struct { path: []const u8, old: []const u8, new: []const u8, completion: []const u8, id: []const u8 },
    edit_lines: struct { path: []const u8, snapshot: []const u8, start: []const u8, end: []const u8, position: Position, content: []const u8, completion: []const u8, id: []const u8 },

    const Position = enum { replace, before, after };

    pub fn parse(kind: []const u8, object: std.json.ObjectMap) !Spec {
        const path = nonEmptyString(object, "path") orelse return error.InvalidEffect;
        const completion_name = nonEmptyString(object, "completion") orelse return error.InvalidEffect;
        const id = nonEmptyString(object, "id") orelse return error.InvalidEffect;
        if (std.mem.eql(u8, kind, "file/read")) return .{ .read = .{ .path = path, .completion = completion_name, .id = id, .start_line = try positiveInteger(object, "start_line", 1, max_bytes), .max_lines = try positiveInteger(object, "max_lines", max_read_lines, max_read_lines), .anchored = if (object.get("anchored")) |value| switch (value) {
            .bool => |flag| flag,
            else => return error.InvalidEffect,
        } else false } };
        if (std.mem.eql(u8, kind, "file/list")) return .{ .list = .{ .path = path, .completion = completion_name, .id = id } };
        const content = string(object, "content") orelse return error.InvalidEffect;
        if (content.len > max_bytes) return error.EffectTooLarge;
        if (std.mem.eql(u8, kind, "file/edit_lines")) {
            const start = nonEmptyString(object, "start") orelse return error.InvalidEffect;
            const snapshot = nonEmptyString(object, "snapshot") orelse return error.InvalidEffect;
            const position = std.meta.stringToEnum(Position, string(object, "position") orelse "replace") orelse return error.InvalidEffect;
            return .{ .edit_lines = .{ .path = path, .snapshot = snapshot, .start = start, .end = string(object, "end") orelse start, .position = position, .content = content, .completion = completion_name, .id = id } };
        }
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
        .read => |read| blk: {
            const source = try std.Io.Dir.cwd().readFileAlloc(io, read.path, allocator, .limited(max_bytes));
            if (!read.anchored) break :blk source;
            defer allocator.free(source);
            break :blk try anchoredPage(allocator, source, read.start_line, read.max_lines);
        },
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
            const result = try editResult(allocator, source, output);
            errdefer allocator.free(result);
            const permissions = (try std.Io.Dir.cwd().statFile(io, edit.path, .{})).permissions;
            var atomic = try std.Io.Dir.cwd().createFileAtomic(io, edit.path, .{ .permissions = permissions, .replace = true });
            defer atomic.deinit(io);
            try atomic.file.writeStreamingAll(io, output);
            try atomic.replace(io);
            break :blk result;
        },
        .edit_lines => |edit| blk: {
            const source = try std.Io.Dir.cwd().readFileAlloc(io, edit.path, allocator, .limited(max_bytes));
            defer allocator.free(source);
            const output = try editLines(allocator, source, edit.snapshot, edit.start, edit.end, edit.position, edit.content);
            defer allocator.free(output);
            // Validate and allocate the response before touching disk. Reporting
            // a formatting failure after a successful write invites unsafe retries.
            const result = try editResult(allocator, source, output);
            errdefer allocator.free(result);
            const permissions = (try std.Io.Dir.cwd().statFile(io, edit.path, .{})).permissions;
            var atomic = try std.Io.Dir.cwd().createFileAtomic(io, edit.path, .{ .permissions = permissions, .replace = true });
            defer atomic.deinit(io);
            try atomic.file.writeStreamingAll(io, output);
            try atomic.replace(io);
            break :blk result;
        },
    };
}

/// A single edit changes one contiguous region. Trim equal whole lines and
/// retain two context lines, comparing newline bytes as well as line contents.
fn editResult(allocator: std.mem.Allocator, before: []const u8, after: []const u8) ![]u8 {
    if (after.len > max_bytes) return error.EffectTooLarge;
    if (std.mem.eql(u8, before, after)) return error.EditDidNotChangeFile;
    const old = try fileLines(allocator, before);
    defer allocator.free(old);
    const new = try fileLines(allocator, after);
    defer allocator.free(new);
    var prefix: usize = 0;
    while (prefix < @min(old.len, new.len) and std.mem.eql(u8, old[prefix], new[prefix])) : (prefix += 1) {}
    var suffix: usize = 0;
    while (suffix < @min(old.len, new.len) - prefix and std.mem.eql(u8, old[old.len - suffix - 1], new[new.len - suffix - 1])) : (suffix += 1) {}
    const first = prefix -| 2;
    const tail = @min(suffix, 2);
    const old_end = old.len - suffix;
    const new_end = new.len - suffix;
    const old_count = old_end + tail - first;
    const new_count = new_end + tail - first;
    var result: std.Io.Writer.Allocating = .init(allocator);
    defer result.deinit();
    try result.writer.print("@@ -{d},{d} +{d},{d} @@\n", .{ first + @intFromBool(old_count != 0), old_count, first + @intFromBool(new_count != 0), new_count });
    for (old[first..prefix]) |line| try diffLine(&result.writer, ' ', line);
    for (old[prefix..old_end]) |line| try diffLine(&result.writer, '-', line);
    for (new[prefix..new_end]) |line| try diffLine(&result.writer, '+', line);
    for (new[new_end .. new_end + tail]) |line| try diffLine(&result.writer, ' ', line);
    const anchors = try anchoredText(allocator, after);
    defer allocator.free(anchors);
    try result.writer.writeByte('\n');
    try result.writer.writeAll(anchors);
    return result.toOwnedSlice();
}

fn fileLines(allocator: std.mem.Allocator, source: []const u8) ![][]const u8 {
    var lines: std.ArrayList([]const u8) = .empty;
    errdefer lines.deinit(allocator);
    var start: usize = 0;
    while (start < source.len) {
        const end = if (std.mem.indexOfScalarPos(u8, source, start, '\n')) |at| at + 1 else source.len;
        try lines.append(allocator, source[start..end]);
        start = end;
    }
    return lines.toOwnedSlice(allocator);
}

fn diffLine(writer: *std.Io.Writer, marker: u8, line: []const u8) !void {
    try writer.writeByte(marker);
    try writer.writeAll(line);
    if (!std.mem.endsWith(u8, line, "\n")) try writer.writeAll("\n\\ No newline at end of file\n");
}

test "edit results include real line numbers, context, and fresh anchors" {
    const a = std.testing.allocator;
    const inserted = try editResult(a, "title\none\ntwo\n", "title\none\ntwo\nadded\n");
    defer a.free(inserted);
    try std.testing.expect(std.mem.startsWith(u8, inserted, "@@ -2,2 +2,3 @@\n one\n two\n+added\n\nsnapshot "));
    try std.testing.expect(std.mem.indexOf(u8, inserted, "4#") != null);
    const replaced = try editResult(a, "alpha", "beta");
    defer a.free(replaced);
    try std.testing.expect(std.mem.startsWith(u8, replaced, "@@ -1,1 +1,1 @@\n-alpha\n\\ No newline at end of file\n+beta\n\\ No newline at end of file\n"));
    const deleted = try editResult(a, "only\n", "");
    defer a.free(deleted);
    try std.testing.expect(std.mem.startsWith(u8, deleted, "@@ -1,1 +0,0 @@\n-only\n"));
    const created = try editResult(a, "", "new\n");
    defer a.free(created);
    try std.testing.expect(std.mem.startsWith(u8, created, "@@ -0,0 +1,1 @@\n+new\n"));
    try std.testing.expectError(error.EditDidNotChangeFile, editResult(a, "same", "same"));
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

fn positiveInteger(object: std.json.ObjectMap, name: []const u8, default: usize, maximum: usize) !usize {
    const value = object.get(name) orelse return default;
    const number = switch (value) {
        .integer => |integer| std.math.cast(usize, integer) orelse return error.InvalidEffect,
        else => return error.InvalidEffect,
    };
    if (number == 0 or number > maximum) return error.InvalidEffect;
    return number;
}

// Snapshot checks cover interior lines as well as the two explicit anchors.
fn snapshotTag(source: []const u8) [16]u8 {
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(source, &digest, .{});
    return std.fmt.bytesToHex(digest[0..8].*, .upper);
}

fn lineTag(source: []const u8) [8]u8 {
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(source, &digest, .{});
    return std.fmt.bytesToHex(digest[0..4].*, .upper);
}

fn anchoredText(allocator: std.mem.Allocator, source: []const u8) ![]u8 {
    return anchoredPage(allocator, source, 1, max_read_lines);
}

fn anchoredPage(allocator: std.mem.Allocator, source: []const u8, start_line: usize, max_lines: usize) ![]u8 {
    if (!std.unicode.utf8ValidateSlice(source) or std.mem.indexOfScalar(u8, source, 0) != null) return error.NotUtf8Text;
    var output: std.Io.Writer.Allocating = .init(allocator);
    errdefer output.deinit();
    const snapshot = snapshotTag(source);
    try output.writer.print("snapshot {s}\n", .{snapshot});
    var lines = std.mem.splitScalar(u8, source, '\n');
    var number: usize = 1;
    while (lines.next()) |line| : (number += 1) {
        if (line.len == 0 and lines.peek() == null and source.len > 0 and source[source.len - 1] == '\n') break;
        if (number < start_line) continue;
        if (number - start_line >= max_lines) {
            try output.writer.print("[more lines: read_file start_line={d}]\n", .{number});
            break;
        }
        const tag = lineTag(line);
        try output.writer.print("{d}#{s}|{s}\n", .{ number, tag, line });
    }
    return output.toOwnedSlice();
}

const Line = struct { start: usize, end: usize, next: usize };

fn anchorLine(source: []const u8, anchor: []const u8) !Line {
    const separator = std.mem.indexOfScalar(u8, anchor, '#') orelse return error.InvalidLineAnchor;
    const number = std.fmt.parseInt(usize, anchor[0..separator], 10) catch return error.InvalidLineAnchor;
    if (number == 0 or anchor.len - separator - 1 != 8) return error.InvalidLineAnchor;
    var offset: usize = 0;
    var current: usize = 1;
    while (true) {
        const end = std.mem.indexOfScalarPos(u8, source, offset, '\n') orelse source.len;
        if (current == number) {
            const tag = lineTag(source[offset..end]);
            if (!std.mem.eql(u8, &tag, anchor[separator + 1 ..])) return error.StaleLineAnchor;
            return .{ .start = offset, .end = end, .next = if (end < source.len) end + 1 else end };
        }
        if (end == source.len or end + 1 == source.len) return error.LineOutOfRange;
        offset = end + 1;
        current += 1;
    }
}

fn editLines(allocator: std.mem.Allocator, source: []const u8, snapshot: []const u8, start: []const u8, end: []const u8, position: Spec.Position, content: []const u8) ![]u8 {
    const current = snapshotTag(source);
    if (!std.mem.eql(u8, &current, snapshot)) return error.StaleSnapshotReadFileAgain;
    if (!std.unicode.utf8ValidateSlice(source) or std.mem.indexOfScalar(u8, source, 0) != null or
        !std.unicode.utf8ValidateSlice(content) or std.mem.indexOfScalar(u8, content, 0) != null) return error.NotUtf8Text;
    const first = try anchorLine(source, start);
    const last = try anchorLine(source, end);
    if (first.start > last.start) return error.ReversedLineRange;
    if (position != .replace and !std.mem.eql(u8, start, end)) return error.InsertionRequiresSingleAnchor;
    // new_text is complete replacement lines without a mandatory final newline.
    // Preserve CRLF and the original final-newline convention outside the edit.
    const newline: []const u8 = if (std.mem.indexOf(u8, source, "\r\n") != null) "\r\n" else "\n";
    // Replacement lines use the file's convention, including internal breaks.
    // Canonicalizing CRLF first avoids duplicating CR in already-correct input.
    const lf = try std.mem.replaceOwned(u8, allocator, content, "\r\n", "\n");
    defer allocator.free(lf);
    const replacement = if (newline.len == 2) try std.mem.replaceOwned(u8, allocator, lf, "\n", "\r\n") else try allocator.dupe(u8, lf);
    defer allocator.free(replacement);
    const from = switch (position) {
        .replace, .before => first.start,
        .after => first.next,
    };
    const to = switch (position) {
        .replace => last.next,
        .before => first.start,
        .after => first.next,
    };
    const prefix_break = position == .after and from == source.len and source.len > 0 and source[source.len - 1] != '\n' and replacement.len > 0;
    const suffix_break = replacement.len > 0 and replacement[replacement.len - 1] != '\n' and (to < source.len or (source.len > 0 and source[source.len - 1] == '\n'));
    const prefix: []const u8 = if (prefix_break) newline else "";
    const suffix: []const u8 = if (suffix_break or (position == .before and replacement.len > 0 and replacement[replacement.len - 1] != '\n')) newline else "";
    if (source.len - (to - from) + replacement.len + prefix.len + suffix.len > max_bytes) return error.EffectTooLarge;
    const output = try std.mem.concat(allocator, u8, &.{ source[0..from], prefix, replacement, suffix, source[to..] });
    if (std.mem.eql(u8, source, output)) {
        allocator.free(output);
        return error.EditDidNotChangeFile;
    }
    return output;
}

test "hashline edits validate snapshots, anchors, ranges and newline boundaries" {
    const a = std.testing.allocator;
    const source = "one\ntwo\nthree\n";
    const snapshot = snapshotTag(source);
    const first = try std.fmt.allocPrint(a, "1#{s}", .{lineTag("one")});
    defer a.free(first);
    const second = try std.fmt.allocPrint(a, "2#{s}", .{lineTag("two")});
    defer a.free(second);
    const preview = try anchoredText(a, source);
    defer a.free(preview);
    try std.testing.expect(std.mem.indexOf(u8, preview, first) != null);
    const replaced = try editLines(a, source, &snapshot, first, second, .replace, "new");
    defer a.free(replaced);
    try std.testing.expectEqualStrings("new\nthree\n", replaced);
    const inserted = try editLines(a, source, &snapshot, second, second, .before, "new");
    defer a.free(inserted);
    try std.testing.expectEqualStrings("one\nnew\ntwo\nthree\n", inserted);
    const removed = try editLines(a, source, &snapshot, second, second, .replace, "");
    defer a.free(removed);
    try std.testing.expectEqualStrings("one\nthree\n", removed);
    try std.testing.expectError(error.StaleSnapshotReadFileAgain, editLines(a, "one\nchanged\nthree\n", &snapshot, first, second, .replace, "new"));
    try std.testing.expectError(error.StaleLineAnchor, editLines(a, source, &snapshot, "1#00000000", second, .replace, "new"));
    try std.testing.expectError(error.ReversedLineRange, editLines(a, source, &snapshot, second, first, .replace, "new"));
    try std.testing.expectError(error.InsertionRequiresSingleAnchor, editLines(a, source, &snapshot, first, second, .after, "new"));
    const tail = try editLines(a, "one", &snapshotTag("one"), first, first, .after, "tail");
    defer a.free(tail);
    try std.testing.expectEqualStrings("one\ntail", tail);
    const empty = try editLines(a, "", &snapshotTag(""), "1#E3B0C442", "1#E3B0C442", .replace, "first");
    defer a.free(empty);
    try std.testing.expectEqualStrings("first", empty);
}

test "hashline pages preserve absolute anchors and bound newline-heavy output" {
    const a = std.testing.allocator;
    const source = try a.alloc(u8, max_bytes);
    defer a.free(source);
    @memset(source, '\n');
    const page = try anchoredText(a, source);
    defer a.free(page);
    try std.testing.expect(page.len < 40_000);
    try std.testing.expect(std.mem.indexOf(u8, page, "[more lines: read_file start_line=2001]") != null);
    const last = try anchoredPage(a, source, max_bytes, 1);
    defer a.free(last);
    try std.testing.expect(std.mem.indexOf(u8, last, "1048576#E3B0C442|") != null);
    try std.testing.expect(std.mem.indexOf(u8, last, "more lines") == null);
    const middle = try anchoredPage(a, "one\ntwo\nthree\n", 2, 1);
    defer a.free(middle);
    try std.testing.expect(std.mem.indexOf(u8, middle, "2#3FC4CCFE|two") != null);
    try std.testing.expect(std.mem.indexOf(u8, middle, "|one") == null);
    try std.testing.expect(std.mem.indexOf(u8, middle, "start_line=3") != null);
}

test "hashline edits reject invalid source and preserve multiline CRLF" {
    const a = std.testing.allocator;
    const source = "one\r\ntwo\r\n";
    const first = try std.fmt.allocPrint(a, "1#{s}", .{lineTag("one\r")});
    defer a.free(first);
    const changed = try editLines(a, source, &snapshotTag(source), first, first, .replace, "first\nsecond\r\n");
    defer a.free(changed);
    try std.testing.expectEqualStrings("first\r\nsecond\r\ntwo\r\n", changed);
    for ([_][]const u8{ "one\n\xff", "one\n\x00" }) |invalid| {
        try std.testing.expectError(error.NotUtf8Text, editLines(a, invalid, &snapshotTag(invalid), "1#7692C3AD", "1#7692C3AD", .replace, "changed"));
    }
}

test "native read paging contract validates numeric limits" {
    const a = std.testing.allocator;
    var parsed = try std.json.parseFromSlice(std.json.Value, a,
        \\{"path":"file","completion":"done","id":"read","anchored":true,"start_line":2001,"max_lines":10}
    , .{});
    defer parsed.deinit();
    const spec = try Spec.parse("file/read", parsed.value.object);
    try std.testing.expect(spec.read.anchored);
    try std.testing.expectEqual(@as(usize, 2001), spec.read.start_line);
    try std.testing.expectEqual(@as(usize, 10), spec.read.max_lines);
    try parsed.value.object.put(a, "max_lines", .{ .integer = 2001 });
    try std.testing.expectError(error.InvalidEffect, Spec.parse("file/read", parsed.value.object));
    try parsed.value.object.put(a, "max_lines", .{ .float = 1.5 });
    try std.testing.expectError(error.InvalidEffect, Spec.parse("file/read", parsed.value.object));
}
