//! Kitty graphics protocol projection. Image payloads are cached independently
//! of text frames, and new cache state is adopted only after successful output.
//! Protocol: https://sw.kovidgoyal.net/kitty/graphics-protocol/
const std = @import("std");

pub fn supported(environ: *const std.process.Environ.Map) bool {
    // Multiplexer passthrough requires its own explicit transport. Do not infer
    // support from an inherited TERM_PROGRAM through tmux/screen.
    if (environ.get("TMUX") != null or environ.get("STY") != null) return false;
    const term = environ.get("TERM") orelse "";
    const program = environ.get("TERM_PROGRAM") orelse "";
    return std.mem.eql(u8, term, "xterm-ghostty") or std.mem.eql(u8, term, "xterm-kitty") or
        std.ascii.eqlIgnoreCase(program, "ghostty") or std.ascii.eqlIgnoreCase(program, "wezterm") or
        environ.get("KITTY_WINDOW_ID") != null;
}

const Entry = struct {
    id: u32,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    row: u32,
    column: u32,
    data: []const u8,
    hash: u64,

    fn samePixels(self: Entry, other: Entry) bool {
        return self.width == other.width and self.height == other.height and self.hash == other.hash;
    }
    fn samePlacement(self: Entry, other: Entry) bool {
        return self.columns == other.columns and self.rows == other.rows and self.row == other.row and self.column == other.column;
    }
};

pub const Cache = struct {
    entries: std.ArrayList(Entry) = .empty,

    pub fn deinit(self: *Cache, allocator: std.mem.Allocator) void {
        for (self.entries.items) |entry| allocator.free(entry.data);
        self.entries.deinit(allocator);
        self.* = .{};
    }

    fn find(self: *const Cache, id: u32) ?Entry {
        for (self.entries.items) |entry| if (entry.id == id) return entry;
        return null;
    }

    /// Replay is only needed after alternate-screen loss (terminal handoff).
    pub fn replay(self: *const Cache, out: *std.ArrayList(u8), allocator: std.mem.Allocator) !void {
        if (self.entries.items.len == 0) return;
        try out.appendSlice(allocator, "\x1b7");
        for (self.entries.items) |entry| {
            try transmit(out, allocator, entry);
            try place(out, allocator, entry);
        }
        try out.appendSlice(allocator, "\x1b8");
    }

    pub fn delete(self: *const Cache, out: *std.ArrayList(u8), allocator: std.mem.Allocator) !void {
        for (self.entries.items) |entry| try remove(out, allocator, entry.id);
    }
};

pub const Plan = struct {
    next: Cache = .{},
    bytes: std.ArrayList(u8) = .empty,
    changed: bool = false,

    pub fn deinit(self: *Plan, allocator: std.mem.Allocator) void {
        self.next.deinit(allocator);
        self.bytes.deinit(allocator);
        self.* = .{};
    }

    pub fn commit(self: *Plan, cache: *Cache, allocator: std.mem.Allocator) void {
        cache.deinit(allocator);
        cache.* = self.next;
        self.next = .{};
    }
};

fn specs(view: std.json.Value, columns: usize, rows: usize) !Specs {
    if (view != .object) return error.InvalidView;
    const lines = view.object.get("lines") orelse return error.InvalidView;
    if (lines != .array) return error.InvalidView;
    return .{ .lines = lines.array.items, .columns = columns, .rows = rows };
}

const Specs = struct {
    lines: []const std.json.Value,
    columns: usize,
    rows: usize,
    index: usize = 0,
    ids: [32]u32 = undefined,
    count: usize = 0,

    fn next(self: *Specs) !?Entry {
        while (self.index < self.lines.len) {
            const index = self.index;
            const line = self.lines[index];
            self.index += 1;
            if (line != .object) return error.InvalidView;
            const value = line.object.get("image") orelse continue;
            if (value == .null) continue;
            if (value != .object or self.count >= 32) return error.InvalidView;
            const object = value.object;
            const format = object.get("format") orelse return error.InvalidView;
            if (format != .string or !std.mem.eql(u8, format.string, "rgba")) return error.InvalidView;
            const data = object.get("data") orelse return error.InvalidView;
            if (data != .string) return error.InvalidView;
            const width = try positive(object, "width", 480);
            const height = try positive(object, "height", 320);
            const image_columns = try positive(object, "columns", 65535);
            const image_rows = try positive(object, "rows", 65535);
            const column = if (object.get("column") != null) try positive(object, "column", 65535) else 1;
            try validateBase64(data.string, @as(usize, width) * height * 4);
            const id = try positive(object, "id", std.math.maxInt(u32));
            for (self.ids[0..self.count]) |seen| if (seen == id) return error.InvalidView;
            // A partially visible header cannot anchor a correctly clipped image.
            // Keep the textual fallback; Lua can reserve fewer rows on small views.
            if (index >= self.rows or image_rows > self.rows - index or column > self.columns or image_columns > self.columns - column + 1) continue;
            self.ids[self.count] = id;
            self.count += 1;
            return .{
                .id = id,
                .width = width,
                .height = height,
                .columns = image_columns,
                .rows = image_rows,
                .row = @intCast(index + 1),
                .column = column,
                .data = data.string,
                .hash = 0,
            };
        }
        return null;
    }
};

pub fn validate(view: std.json.Value, columns: usize, rows: usize) !void {
    var iterator = try specs(view, columns, rows);
    while (try iterator.next()) |_| {}
}

pub fn prepare(allocator: std.mem.Allocator, cache: *const Cache, view: std.json.Value, columns: usize, rows: usize) !Plan {
    var result: Plan = .{};
    errdefer result.deinit(allocator);
    var iterator = try specs(view, columns, rows);
    while (try iterator.next()) |spec| {
        var entry = spec;
        entry.data = try allocator.dupe(u8, spec.data);
        entry.hash = std.hash.Wyhash.hash(0, spec.data);
        result.next.entries.append(allocator, entry) catch |err| {
            allocator.free(entry.data);
            return err;
        };
    }
    for (cache.entries.items) |old| if (result.next.find(old.id) == null) {
        try remove(&result.bytes, allocator, old.id);
    };
    for (result.next.entries.items) |entry| {
        const old = cache.find(entry.id);
        const pixels_changed = if (old) |previous| !entry.samePixels(previous) else true;
        const moved = if (old) |previous| !entry.samePlacement(previous) else true;
        if (pixels_changed) {
            if (old != null) try remove(&result.bytes, allocator, entry.id);
            try transmit(&result.bytes, allocator, entry);
        }
        if (pixels_changed or moved) {
            try result.bytes.appendSlice(allocator, "\x1b7");
            try place(&result.bytes, allocator, entry);
            try result.bytes.appendSlice(allocator, "\x1b8");
        }
    }
    result.changed = result.bytes.items.len != 0;
    return result;
}

fn transmit(out: *std.ArrayList(u8), allocator: std.mem.Allocator, entry: Entry) !void {
    var offset: usize = 0;
    while (offset < entry.data.len) {
        const end = @min(offset + 4096, entry.data.len);
        if (offset == 0) try out.print(allocator, "\x1b_Ga=t,t=d,f=32,s={d},v={d},i={d},q=2,m={d};", .{ entry.width, entry.height, entry.id, @intFromBool(end != entry.data.len) }) else try out.print(allocator, "\x1b_Gm={d};", .{@intFromBool(end != entry.data.len)});
        try out.appendSlice(allocator, entry.data[offset..end]);
        try out.appendSlice(allocator, "\x1b\\");
        offset = end;
    }
}

fn place(out: *std.ArrayList(u8), allocator: std.mem.Allocator, entry: Entry) !void {
    try out.print(allocator, "\x1b[{d};{d}H\x1b_Ga=p,i={d},p=1,c={d},r={d},C=1,q=2;\x1b\\", .{ entry.row, entry.column, entry.id, entry.columns, entry.rows });
}

fn remove(out: *std.ArrayList(u8), allocator: std.mem.Allocator, id: u32) !void {
    try out.print(allocator, "\x1b_Ga=d,d=I,i={d},q=2;\x1b\\", .{id});
}

fn positive(object: std.json.ObjectMap, key: []const u8, maximum: u32) !u32 {
    const value = object.get(key) orelse return error.InvalidView;
    if (value != .integer or value.integer < 1 or value.integer > maximum) return error.InvalidView;
    return @intCast(value.integer);
}

fn validateBase64(data: []const u8, bytes: usize) !void {
    if (data.len != std.base64.standard.Encoder.calcSize(bytes)) return error.InvalidView;
    const padding = (3 - bytes % 3) % 3;
    for (data[0 .. data.len - padding]) |byte| {
        if (!std.ascii.isAlphanumeric(byte) and byte != '+' and byte != '/') return error.InvalidView;
    }
    for (data[data.len - padding ..]) |byte| if (byte != '=') return error.InvalidView;
}

test "graphics plans cache pixels, move placements, and delete hidden images" {
    const a = std.testing.allocator;
    var parsed = try std.json.parseFromSlice(std.json.Value, a,
        \\{"lines":[{"spans":[],"image":{"id":1,"format":"rgba","data":"yGQU/w==","width":1,"height":1,"columns":4,"rows":1}}]}
    , .{});
    defer parsed.deinit();
    var cache: Cache = .{};
    defer cache.deinit(a);
    var first = try prepare(a, &cache, parsed.value, 80, 24);
    defer first.deinit(a);
    try std.testing.expect(first.changed);
    try std.testing.expect(std.mem.indexOf(u8, first.bytes.items, "a=t,t=d,f=32") != null);
    try std.testing.expect(std.mem.indexOf(u8, first.bytes.items, "C=1,q=2") != null);
    // Preparing a superseded frame does not advance the terminal cache.
    try std.testing.expectEqual(@as(usize, 0), cache.entries.items.len);
    first.commit(&cache, a);
    var unchanged = try prepare(a, &cache, parsed.value, 80, 24);
    defer unchanged.deinit(a);
    try std.testing.expect(!unchanged.changed and unchanged.bytes.items.len == 0);
    try parsed.value.object.getPtr("lines").?.array.items[0].object.getPtr("image").?.object.put(a, "columns", .{ .integer = 5 });
    var moved = try prepare(a, &cache, parsed.value, 80, 24);
    defer moved.deinit(a);
    try std.testing.expect(moved.changed and std.mem.indexOf(u8, moved.bytes.items, "a=t") == null);
    moved.commit(&cache, a);
    var replay: std.ArrayList(u8) = .empty;
    defer replay.deinit(a);
    try cache.replay(&replay, a);
    try std.testing.expect(std.mem.indexOf(u8, replay.items, "a=t") != null);
    // A view too narrow to contain the placement removes it completely.
    var clipped = try prepare(a, &cache, parsed.value, 1, 24);
    defer clipped.deinit(a);
    try std.testing.expect(std.mem.indexOf(u8, clipped.bytes.items, "a=d,d=I,i=1") != null);
    clipped.commit(&cache, a);
    try std.testing.expectEqual(@as(usize, 0), cache.entries.items.len);
}

test "image payload validation rejects control injection and inconsistent dimensions" {
    try validateBase64("yGQU/w==", 4);
    try std.testing.expectError(error.InvalidView, validateBase64("yGQ\x1b/w==", 4));
    try std.testing.expectError(error.InvalidView, validateBase64("yGQU/w==", 8));
}

test "image validation preserves clipped IDs and requires no payload allocation" {
    const allocator = std.testing.allocator;
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator,
        \\{"lines":[{"spans":[],"image":{"id":1,"format":"rgba","data":"yGQU/w==","width":1,"height":1,"columns":4,"rows":1}},{"spans":[],"image":{"id":1,"format":"rgba","data":"yGQU/w==","width":1,"height":1,"columns":1,"rows":1}}]}
    , .{});
    defer parsed.deinit();
    // A clipped first occurrence does not reserve an ID.
    try validate(parsed.value, 3, 2);
    const cache: Cache = .{};
    var plan = try prepare(allocator, &cache, parsed.value, 3, 2);
    defer plan.deinit(allocator);
    try std.testing.expectEqual(@as(usize, 1), plan.next.entries.items.len);
    // Once visible, the first occurrence reserves its ID, including against
    // subsequent clipped placements.
    try std.testing.expectError(error.InvalidView, validate(parsed.value, 4, 2));
    try std.testing.expectError(error.InvalidView, prepare(allocator, &cache, parsed.value, 4, 2));
}
