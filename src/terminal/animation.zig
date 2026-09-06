//! Compiled, fixed-cell animation slots sampled from a native monotonic clock.
//! At most 128 visible slots and 1 MiB of compiled IDs/payloads per plan.
const std = @import("std");
const presenter = @import("presenter.zig");
const width = @import("width.zig");
pub const max_slots = 128;
pub const max_compiled_bytes = 1024 * 1024;
const Frame = struct { start: usize, len: usize };
const Slot = struct {
    id: []u8,
    row: usize,
    column: usize,
    interval: i96,
    phase: usize,
    frames: std.ArrayList(Frame) = .empty,
    epoch: i96 = 0,
    last: ?usize = null,

    fn deinit(self: *Slot, allocator: std.mem.Allocator) void {
        allocator.free(self.id);
        self.frames.deinit(allocator);
    }
    fn sample(self: Slot, now: i96) usize {
        const ticks = @divFloor(@max(0, now - self.epoch), self.interval);
        return @intCast(@mod(ticks + self.phase, self.frames.items.len));
    }
};

pub const Plan = struct {
    slots: std.ArrayList(Slot) = .empty,
    bytes: std.ArrayList(u8) = .empty,
    sampled_at: i96 = 0,

    pub fn deinit(self: *Plan, allocator: std.mem.Allocator) void {
        for (self.slots.items) |*slot| slot.deinit(allocator);
        self.slots.deinit(allocator);
        self.bytes.deinit(allocator);
        self.* = .{};
    }
    fn payload(self: *const Plan, slot: Slot, index: usize) []const u8 {
        const frame = slot.frames.items[index];
        return self.bytes.items[frame.start..][0..frame.len];
    }
    fn sameSlot(self: *const Plan, slot: Slot, old: *const Plan, previous: Slot) bool {
        if (!std.mem.eql(u8, slot.id, previous.id) or slot.row != previous.row or slot.column != previous.column or slot.interval != previous.interval or slot.phase != previous.phase or slot.frames.items.len != previous.frames.items.len) return false;
        for (0..slot.frames.items.len) |index| if (!std.mem.eql(u8, self.payload(slot, index), old.payload(previous, index))) return false;
        return true;
    }
    /// Preserve each semantic ID's clock, even if placement or frame data changes.
    /// Identical slots also retain the last successfully painted frame.
    pub fn reconcile(self: *Plan, old: *const Plan, now: i96) void {
        self.sampled_at = now;
        for (self.slots.items) |*slot| {
            slot.epoch = now;
            slot.last = null;
            for (old.slots.items) |previous| {
                if (std.mem.eql(u8, slot.id, previous.id)) {
                    slot.epoch = previous.epoch;
                    break;
                }
            }
            for (old.slots.items) |previous| {
                if (self.sameSlot(slot.*, old, previous)) {
                    slot.last = previous.last;
                    break;
                }
            }
        }
    }
    pub fn changed(self: *const Plan, old: *const Plan) bool {
        if (self.slots.items.len != old.slots.items.len) return true;
        for (self.slots.items, old.slots.items) |slot, previous| if (!self.sameSlot(slot, old, previous)) return true;
        return false;
    }
    /// Append only position and paint payload. The terminal owns synchronized
    /// output and cursor save/restore. Sampling never mutates committed state.
    pub fn append(self: *const Plan, out: *std.ArrayList(u8), allocator: std.mem.Allocator, now: i96, force: bool) !bool {
        var emitted = false;
        for (self.slots.items) |slot| {
            const index = slot.sample(now);
            const bytes = self.payload(slot, index);
            if (!force) if (slot.last) |last| if (std.mem.eql(u8, bytes, self.payload(slot, last))) continue;
            try out.print(allocator, "\x1b[{d};{d}H", .{ slot.row, slot.column });
            try out.appendSlice(allocator, bytes);
            emitted = true;
        }
        return emitted;
    }
    /// Call only after the appended patches have been successfully written.
    /// Also commit no-op samples to advance past repeated identical frames.
    pub fn commit(self: *Plan, now: i96) void {
        self.sampled_at = now;
        for (self.slots.items) |*slot| slot.last = slot.sample(now);
    }
    pub fn nextDeadline(self: *const Plan) ?i96 {
        var deadline: ?i96 = null;
        for (self.slots.items) |slot| {
            if (slot.last == null) return self.sampled_at;
            const current = slot.sample(self.sampled_at);
            if (!std.mem.eql(u8, self.payload(slot, current), self.payload(slot, slot.last.?))) return self.sampled_at;
            const tick = @divFloor(@max(0, self.sampled_at - slot.epoch), slot.interval);
            for (1..slot.frames.items.len) |step| {
                const next = (current + step) % slot.frames.items.len;
                if (std.mem.eql(u8, self.payload(slot, current), self.payload(slot, next))) continue;
                const at = slot.epoch + (tick + @as(i96, @intCast(step))) * slot.interval;
                deadline = if (deadline) |previous| @min(previous, at) else at;
                break;
            }
        }
        return deadline;
    }
};

pub fn validate(allocator: std.mem.Allocator, view: std.json.Value, columns: usize) !void {
    try compile(allocator, view, columns, null);
}
pub fn build(allocator: std.mem.Allocator, view: std.json.Value, columns: usize) !Plan {
    var result: Plan = .{};
    errdefer result.deinit(allocator);
    try compile(allocator, view, columns, &result);
    return result;
}
fn integer(value: ?std.json.Value, low: i64, high: i64) !usize {
    const item = value orelse return error.InvalidView;
    if (item != .integer or item.integer < low or item.integer > high) return error.InvalidView;
    return @intCast(item.integer);
}
fn columnAt(text: []const u8, target: usize) !usize {
    var offset: usize = 0;
    var cells: usize = 0;
    while (offset < target) {
        const cluster = width.nextCluster(text, offset) catch return error.InvalidView;
        if (cluster.end > target) return error.InvalidView;
        offset = cluster.end;
        cells += cluster.width;
    }
    return cells;
}
fn compile(allocator: std.mem.Allocator, view: std.json.Value, columns: usize, plan: ?*Plan) !void {
    if (view != .object) return error.InvalidView;
    const lines = view.object.get("lines") orelse return error.InvalidView;
    if (lines != .array) return error.InvalidView;
    var compiled_bytes: usize = 0;
    var visible_slots: usize = 0;
    var text: std.ArrayList(u8) = .empty;
    defer text.deinit(allocator);
    var replacement: std.ArrayList(u8) = .empty;
    defer replacement.deinit(allocator);
    for (lines.array.items, 1..) |line, row| {
        if (line != .object) return error.InvalidView;
        const spans = line.object.get("spans") orelse return error.InvalidView;
        if (spans != .array) return error.InvalidView;
        var animated = false;
        for (spans.array.items) |span| {
            if (span != .object) return error.InvalidView;
            if (span.object.contains("animation")) animated = true;
        }
        if (!animated) continue;
        text.clearRetainingCapacity();
        for (spans.array.items) |span| {
            const base = span.object.get("text") orelse return error.InvalidView;
            if (base != .string) return error.InvalidView;
            try presenter.validateText(base.string);
            try text.appendSlice(allocator, base.string);
        }
        var offset: usize = 0;
        for (spans.array.items) |span| {
            const base = span.object.get("text").?.string;
            const start = offset;
            offset += base.len;
            const descriptor = span.object.get("animation") orelse continue;
            if (descriptor != .object) return error.InvalidView;
            var fields = descriptor.object.iterator();
            while (fields.next()) |field| {
                const name = field.key_ptr.*;
                if (!std.mem.eql(u8, name, "id") and !std.mem.eql(u8, name, "interval_ms") and !std.mem.eql(u8, name, "phase") and !std.mem.eql(u8, name, "frames")) return error.InvalidView;
            }
            const id = descriptor.object.get("id") orelse return error.InvalidView;
            if (id != .string or id.string.len == 0 or id.string.len > 256) return error.InvalidView;
            try presenter.validateText(id.string);
            const interval = try integer(descriptor.object.get("interval_ms"), 10, 60000);
            const frames = descriptor.object.get("frames") orelse return error.InvalidView;
            if (frames != .array or frames.array.items.len == 0 or frames.array.items.len > 64) return error.InvalidView;
            const phase = if (descriptor.object.get("phase")) |value| try integer(value, 0, @intCast(frames.array.items.len - 1)) else 0;
            const column = try columnAt(text.items, start);
            const end_column = try columnAt(text.items, offset);
            const cells = end_column - column;
            if (cells == 0) return error.InvalidView;
            const visible = column < columns and end_column <= columns;
            if (visible) {
                visible_slots += 1;
                if (visible_slots > max_slots) return error.InvalidView;
                compiled_bytes += id.string.len;
            }
            var slot: ?Slot = if (visible and plan != null) .{
                .id = try allocator.dupe(u8, id.string),
                .row = row,
                .column = column + 1,
                .interval = @as(i96, @intCast(interval)) * std.time.ns_per_ms,
                .phase = phase,
            } else null;
            errdefer if (slot) |*owned| owned.deinit(allocator);
            for (frames.array.items) |frame| {
                const payload = try presenter.spanFrame(span, frame);
                if ((width.textWidth(payload.text) catch return error.InvalidView) != cells) return error.InvalidView;
                const first_cluster = width.nextCluster(payload.text, 0) catch return error.InvalidView;
                if (first_cluster.width == 0) return error.InvalidView;
                // Alternate text must preserve boundaries with its neighbors
                // too: a leading combining mark or trailing joiner could
                // otherwise repaint cells outside this slot.
                if (!std.mem.eql(u8, payload.text, base)) {
                    replacement.clearRetainingCapacity();
                    try replacement.appendSlice(allocator, text.items[0..start]);
                    try replacement.appendSlice(allocator, payload.text);
                    const replacement_end = replacement.items.len;
                    try replacement.appendSlice(allocator, text.items[offset..]);
                    const first_cell = try columnAt(replacement.items, start);
                    const end_cell = try columnAt(replacement.items, replacement_end);
                    if (first_cell != column or end_cell - first_cell != cells) return error.InvalidView;
                }
                if (visible) {
                    compiled_bytes = std.math.add(usize, compiled_bytes, payload.byteLength()) catch return error.InvalidView;
                    if (compiled_bytes > max_compiled_bytes) return error.InvalidView;
                }
                if (slot) |*owned| {
                    const beginning = plan.?.bytes.items.len;
                    try payload.append(&plan.?.bytes, allocator);
                    try owned.frames.append(allocator, .{ .start = beginning, .len = plan.?.bytes.items.len - beginning });
                }
            }
            if (slot) |owned| {
                try plan.?.slots.append(allocator, owned);
                slot = null;
            }
        }
    }
}

fn testPlan(source: []const u8, columns: usize) !Plan {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, source, .{});
    defer parsed.deinit();
    try validate(std.testing.allocator, parsed.value, columns);
    return build(std.testing.allocator, parsed.value, columns);
}
const clock_fixture =
    \\{"lines":[{"spans":[{"text":"-","animation":{"id":"spinner","interval_ms":10,"frames":[{"text":"-"},{"text":"+"},{"text":"*"}]}}]}]}
;
test "native clock preserves phase across views and skips elapsed ticks without mutation before commit" {
    const allocator = std.testing.allocator;
    const ms = std.time.ns_per_ms;
    var old = try testPlan(clock_fixture, 80);
    defer old.deinit(allocator);
    const empty: Plan = .{};
    old.reconcile(&empty, 100 * ms);
    var out: std.ArrayList(u8) = .empty;
    defer out.deinit(allocator);
    try std.testing.expect(try old.append(&out, allocator, 100 * ms, false));
    try std.testing.expect(old.slots.items[0].last == null);
    old.commit(100 * ms);
    try std.testing.expectEqual(@as(?i96, 110 * ms), old.nextDeadline());
    out.clearRetainingCapacity();
    try std.testing.expect(try old.append(&out, allocator, 145 * ms, false));
    try std.testing.expect(std.mem.indexOf(u8, out.items, "+") != null);
    try std.testing.expectEqual(@as(?usize, 0), old.slots.items[0].last);
    old.commit(145 * ms);
    try std.testing.expectEqual(@as(?i96, 150 * ms), old.nextDeadline());
    var next = try testPlan(clock_fixture, 80);
    defer next.deinit(allocator);
    next.reconcile(&old, 146 * ms);
    try std.testing.expect(!next.changed(&old));
    try std.testing.expectEqual(@as(i96, 100 * ms), next.slots.items[0].epoch);
    out.clearRetainingCapacity();
    try std.testing.expect(!try next.append(&out, allocator, 146 * ms, false));
    try std.testing.expect(empty.changed(&next));
    // No tick allocation is necessary after the caller reserves its output.
    out.clearRetainingCapacity();
    try std.testing.expect(try next.append(&out, std.testing.failing_allocator, 151 * ms, false));
    next.commit(151 * ms);
}

test "same ID shares epoch and supports phase offsets while equal frames sleep" {
    const allocator = std.testing.allocator;
    var plan = try testPlan(
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{"text":"x"},{"text":"x"},{"text":"y"}]}},{"text":"x","animation":{"id":"s","interval_ms":10,"phase":2,"frames":[{"text":"x"},{"text":"x"},{"text":"y"}]}}]}]}
    , 80);
    defer plan.deinit(allocator);
    const empty: Plan = .{};
    plan.reconcile(&empty, 77);
    try std.testing.expectEqual(@as(usize, 0), plan.slots.items[0].sample(77));
    try std.testing.expectEqual(@as(usize, 2), plan.slots.items[1].sample(77));
    plan.commit(77);
    try std.testing.expectEqual(@as(?i96, 10 * std.time.ns_per_ms + 77), plan.nextDeadline());
    var repeated = try testPlan(
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{},{},{}]}}]}]}
    , 80);
    defer repeated.deinit(allocator);
    repeated.reconcile(&empty, 0);
    repeated.commit(0);
    try std.testing.expect(repeated.nextDeadline() == null);
    var duplicate_then_change = try testPlan(
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{},{},{"text":"y"}]}}]}]}
    , 80);
    defer duplicate_then_change.deinit(allocator);
    duplicate_then_change.reconcile(&empty, 0);
    duplicate_then_change.commit(0);
    try std.testing.expectEqual(@as(?i96, 20 * std.time.ns_per_ms), duplicate_then_change.nextDeadline());
}

test "frame overrides preserve links and base style with equal cell width across UTF8 lengths" {
    const allocator = std.testing.allocator;
    var plan = try testPlan(
        \\{"lines":[{"spans":[{"text":"a","style":{"bold":true,"foreground":"red"},"link":"https://example.test","animation":{"id":"color","interval_ms":20,"frames":[{}, {"text":"é","style":{"foreground":"green"}}]}}]}]}
    , 1);
    defer plan.deinit(allocator);
    const slot = plan.slots.items[0];
    const payload = plan.payload(slot, 1);
    try std.testing.expect(std.mem.indexOf(u8, payload, "\x1b[0;1;32m") != null);
    try std.testing.expect(std.mem.indexOf(u8, payload, "https://example.test\x1b\\é\x1b]8;;\x1b\\") != null);
    try std.testing.expectEqual(@as(usize, 2), slot.frames.items.len);
}

test "descriptors are validated when clipped and partial wide spans never schedule" {
    const allocator = std.testing.allocator;
    const wide_fixture =
        \\{"lines":[{"spans":[{"text":"界","animation":{"id":"w","interval_ms":10,"frames":[{}, {"text":"ab"}]}}]}]}
    ;
    var clipped = try testPlan(wide_fixture, 1);
    defer clipped.deinit(allocator);
    try std.testing.expectEqual(@as(usize, 0), clipped.slots.items.len);
    var visible = try testPlan(wide_fixture, 2);
    defer visible.deinit(allocator);
    try std.testing.expectEqual(@as(usize, 1), visible.slots.items.len);
    const invalid = [_][]const u8{
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"","interval_ms":10,"frames":[{}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":9,"frames":[{}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"phase":1,"frames":[{}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{"text":"xx"}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{"text":"\u001b"}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{"action":"replacement"}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"e","animation":{"id":"s","interval_ms":10,"frames":[{}]}},{"text":"́"}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"e"},{"text":"́x","animation":{"id":"s","interval_ms":10,"frames":[{}]}}]}]}
    };
    for (invalid) |source| {
        var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source, .{});
        defer parsed.deinit();
        try std.testing.expectError(error.InvalidView, validate(allocator, parsed.value, 0));
        try std.testing.expectError(error.InvalidView, build(allocator, parsed.value, 0));
    }
}

test "visible slot and compiled payload limits agree between validation and build" {
    const allocator = std.testing.allocator;
    var source: std.ArrayList(u8) = .empty;
    defer source.deinit(allocator);
    try source.appendSlice(allocator, "{\"lines\":[{\"spans\":[");
    for (0..129) |index| {
        if (index != 0) try source.append(allocator, ',');
        try source.appendSlice(allocator, "{\"text\":\"x\",\"animation\":{\"id\":\"s\",\"interval_ms\":10,\"frames\":[{}]}}");
    }
    try source.appendSlice(allocator, "]}]}");
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source.items, .{});
    defer parsed.deinit();
    try std.testing.expectError(error.InvalidView, validate(allocator, parsed.value, 129));
    try std.testing.expectError(error.InvalidView, build(allocator, parsed.value, 129));
    var clipped = try build(allocator, parsed.value, 128);
    defer clipped.deinit(allocator);
    try std.testing.expectEqual(@as(usize, 128), clipped.slots.items.len);

    source.clearRetainingCapacity();
    try source.appendSlice(allocator, "{\"lines\":[{\"spans\":[");
    for (0..4) |index| {
        if (index != 0) try source.append(allocator, ',');
        try source.appendSlice(allocator, "{\"text\":\"x\",\"link\":\"");
        try source.appendNTimes(allocator, 'a', 4096);
        try source.appendSlice(allocator, "\",\"animation\":{\"id\":\"s\",\"interval_ms\":10,\"frames\":[");
        for (0..64) |frame| {
            if (frame != 0) try source.append(allocator, ',');
            try source.appendSlice(allocator, "{}");
        }
        try source.appendSlice(allocator, "]}}");
    }
    try source.appendSlice(allocator, "]}]}");
    var oversized = try std.json.parseFromSlice(std.json.Value, allocator, source.items, .{});
    defer oversized.deinit();
    try std.testing.expectError(error.InvalidView, validate(allocator, oversized.value, 4));
    try std.testing.expectError(error.InvalidView, build(allocator, oversized.value, 4));
    var within_limit = try build(allocator, oversized.value, 3);
    defer within_limit.deinit(allocator);
    try std.testing.expect(within_limit.bytes.items.len < max_compiled_bytes);
}

test "alternate frames cannot join graphemes across their fixed slot" {
    const allocator = std.testing.allocator;
    const invalid = [_][]const u8{
        \\{"lines":[{"spans":[{"text":"́x","animation":{"id":"s","interval_ms":10,"frames":[{}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"a"},{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{"text":"́x"}]}}]}]}
        ,
        \\{"lines":[{"spans":[{"text":"x","animation":{"id":"s","interval_ms":10,"frames":[{"text":"́x"}]}}]}]}
    };
    for (invalid) |source| {
        var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source, .{});
        defer parsed.deinit();
        try std.testing.expectError(error.InvalidView, validate(allocator, parsed.value, 80));
        try std.testing.expectError(error.InvalidView, build(allocator, parsed.value, 80));
    }
}
