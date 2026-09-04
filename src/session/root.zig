//! Non-reentrant FIFO owner for Lua transactions and fixed native effects.
const std = @import("std");
const lua = @import("misa_lua_runtime");
const terminal_module = @import("misa_terminal");

pub const Session = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    runtime: *lua.Runtime,
    terminal: *terminal_module.Terminal,
    queue: std.ArrayList([]u8) = .empty,
    queue_head: usize = 0,
    running: bool = false,
    quit: bool = false,
    read_requested: bool = false,

    pub fn deinit(self: *Session) void {
        for (self.queue.items[self.queue_head..]) |item| self.allocator.free(item);
        self.queue.deinit(self.allocator);
    }

    /// Seed app/start, then drain the queue. Dispatch effects only append; they never recurse.
    pub fn run(self: *Session) !void {
        std.debug.assert(!self.running);
        self.running = true;
        defer self.running = false;
        try self.enqueue("{\"type\":\"app/start\"}");
        while (!self.quit and (self.queue_head < self.queue.items.len or self.read_requested)) {
            if (self.queue_head == self.queue.items.len) {
                try self.readTerminal();
                continue;
            }
            const event = self.queue.items[self.queue_head];
            self.queue_head += 1;
            defer self.allocator.free(event);
            var transaction = self.runtime.dispatch(event) catch return error.LuaTransactionFailed;
            defer transaction.deinit();
            const view = transaction.view;
            const effects = transaction.effects;
            // Validate everything, then make the pending semantic view visible
            // before committing policy state. Validation/presentation failures
            // leave canonical Lua db unchanged. After commit, effect I/O is
            // fatal on failure and cannot in general be rolled back.
            if (view != .null) try self.terminal.validatePresentation(view);
            for (effects) |effect| try validateEffect(effect);
            if (view != .null) try self.terminal.present(view);
            try self.runtime.commitTransaction();
            for (effects) |effect| try self.execute(effect);
            self.compactQueue();
            // A read request is level-triggered. Drain already-decoded events
            // before blocking again so a single read containing a whole line
            // cannot starve its Enter event.
            if (!self.quit and self.queue_head == self.queue.items.len and self.read_requested)
                try self.readTerminal();
        }
    }

    fn enqueue(self: *Session, json: []const u8) !void {
        try self.queue.append(self.allocator, try self.allocator.dupe(u8, json));
    }

    fn compactQueue(self: *Session) void {
        if (self.queue_head == 0 or (self.queue_head < 64 and self.queue_head * 2 < self.queue.items.len)) return;
        const remaining = self.queue.items.len - self.queue_head;
        std.mem.copyForwards([]u8, self.queue.items[0..remaining], self.queue.items[self.queue_head..]);
        self.queue.items.len = remaining;
        self.queue_head = 0;
    }

    fn execute(self: *Session, value: std.json.Value) !void {
        const object = switch (value) {
            .object => |o| o,
            else => return error.InvalidEffect,
        };
        const kind = stringField(object, "type") orelse return error.InvalidEffect;
        if (std.mem.eql(u8, kind, "dispatch")) {
            const event = object.get("event") orelse return error.InvalidEffect;
            const json = try std.json.Stringify.valueAlloc(self.allocator, event, .{});
            defer self.allocator.free(json);
            try self.enqueue(json);
        } else if (std.mem.eql(u8, kind, "terminal/read")) {
            self.read_requested = true;
        } else if (std.mem.eql(u8, kind, "view/commit")) {
            try self.terminal.commit(object.get("lines") orelse return error.InvalidEffect);
        } else if (std.mem.eql(u8, kind, "app/quit")) {
            self.quit = true;
        } else if (std.mem.eql(u8, kind, "process/run")) {
            try self.runProcess(object);
        } else return error.UnknownNativeEffect;
    }

    fn readTerminal(self: *Session) !void {
        self.read_requested = false;
        var events: std.ArrayList(terminal_module.Event) = .empty;
        defer {
            for (events.items) |event| event.deinit(self.allocator);
            events.deinit(self.allocator);
        }
        try self.terminal.readEvents(&events);
        for (events.items) |event| try self.enqueueInput(event);
        if (events.items.len == 0) self.read_requested = true;
    }

    fn runProcess(self: *Session, object: std.json.ObjectMap) !void {
        if (object.get("stdin") != null) return error.UnsupportedProcessStdin;
        const values = switch (object.get("argv") orelse return error.InvalidEffect) {
            .array => |a| a.items,
            else => return error.InvalidEffect,
        };
        if (values.len == 0) return error.InvalidEffect;
        var argv: std.ArrayList([]const u8) = .empty;
        defer argv.deinit(self.allocator);
        for (values) |value| {
            const arg = switch (value) {
                .string => |s| s,
                else => return error.InvalidEffect,
            };
            if (arg.len == 0 or std.mem.indexOfScalar(u8, arg, 0) != null) return error.InvalidEffect;
            try argv.append(self.allocator, arg);
        }
        const completion = stringField(object, "completion") orelse return error.InvalidEffect;
        const id = stringField(object, "id") orelse return error.InvalidEffect;
        const result = std.process.run(self.allocator, self.io, .{ .argv = argv.items, .stdout_limit = .limited(1024 * 1024), .stderr_limit = .limited(1024 * 1024) }) catch |err| {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{ .type = completion, .id = id, .ok = false, .stdout = "", .stderr = @errorName(err), .status = @as(i32, -1) }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
            return;
        };
        defer self.allocator.free(result.stdout);
        defer self.allocator.free(result.stderr);
        const status: i32 = switch (result.term) {
            .exited => |code| code,
            .signal => |sig| -@as(i32, @intCast(@intFromEnum(sig))),
            .stopped => |sig| -@as(i32, @intCast(@intFromEnum(sig))),
            .unknown => |code| @intCast(code),
        };
        const stdout = try sanitizeProcessOutput(self.allocator, result.stdout, 1024 * 1024);
        defer self.allocator.free(stdout);
        const stderr = try sanitizeProcessOutput(self.allocator, result.stderr, 1024 * 1024);
        defer self.allocator.free(stderr);
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{ .type = completion, .id = id, .ok = status == 0, .stdout = stdout, .stderr = stderr, .status = status }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
    }

    fn enqueueInput(self: *Session, event: terminal_module.Event) !void {
        const json = switch (event) {
            .text => |text| try std.json.Stringify.valueAlloc(self.allocator, .{ .type = "terminal/input", .kind = "text", .text = text }, .{}),
            .enter => try inputJson(self.allocator, "enter"),
            .backspace => try inputJson(self.allocator, "backspace"),
            .arrow_up => try inputJson(self.allocator, "arrow_up"),
            .arrow_down => try inputJson(self.allocator, "arrow_down"),
            .arrow_left => try inputJson(self.allocator, "arrow_left"),
            .arrow_right => try inputJson(self.allocator, "arrow_right"),
            .escape => try inputJson(self.allocator, "escape"),
            .ctrl_c => try inputJson(self.allocator, "ctrl_c"),
            .eof => try inputJson(self.allocator, "eof"),
        };
        defer self.allocator.free(json);
        try self.enqueue(json);
    }
};
pub fn validateEffect(value: std.json.Value) !void {
    const object = switch (value) {
        .object => |o| o,
        else => return error.InvalidEffect,
    };
    const kind = nonEmptyStringField(object, "type") orelse return error.InvalidEffect;
    if (std.mem.eql(u8, kind, "dispatch")) {
        const event = switch (object.get("event") orelse return error.InvalidEffect) {
            .object => |o| o,
            else => return error.InvalidEffect,
        };
        _ = nonEmptyStringField(event, "type") orelse return error.InvalidEffect;
    } else if (std.mem.eql(u8, kind, "terminal/read") or std.mem.eql(u8, kind, "app/quit")) {
        return;
    } else if (std.mem.eql(u8, kind, "view/commit")) {
        try terminal_module.validateLines(object.get("lines") orelse return error.InvalidEffect);
    } else if (std.mem.eql(u8, kind, "process/run")) {
        if (object.get("stdin") != null) return error.UnsupportedProcessStdin;
        const argv = switch (object.get("argv") orelse return error.InvalidEffect) {
            .array => |a| a.items,
            else => return error.InvalidEffect,
        };
        if (argv.len == 0) return error.InvalidEffect;
        for (argv) |arg| switch (arg) {
            .string => |string| if (string.len == 0 or std.mem.indexOfScalar(u8, string, 0) != null) return error.InvalidEffect,
            else => return error.InvalidEffect,
        };
        _ = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect;
        _ = nonEmptyStringField(object, "id") orelse return error.InvalidEffect;
    } else return error.UnknownNativeEffect;
}

/// Make captured process bytes safe for JSON and later terminal presentation.
pub fn sanitizeProcessOutput(allocator: std.mem.Allocator, input: []const u8, limit: usize) ![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    var i: usize = 0;
    while (i < input.len and out.items.len < limit) {
        const byte = input[i];
        if (byte == 0x1b) {
            i += 1;
            if (i < input.len and input[i] == '[') {
                i += 1;
                while (i < input.len) : (i += 1) if (input[i] >= 0x40 and input[i] <= 0x7e) {
                    i += 1;
                    break;
                };
            } else if (i < input.len and input[i] == ']') {
                i += 1;
                while (i < input.len) {
                    if (input[i] == 7) {
                        i += 1;
                        break;
                    }
                    if (input[i] == 0x1b and i + 1 < input.len and input[i + 1] == '\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else if (i < input.len) i += 1;
            continue;
        }
        if (byte == '\r') {
            if (out.items.len < limit) try out.append(allocator, '\n');
            i += 1;
            if (i < input.len and input[i] == '\n') i += 1;
            continue;
        }
        if (byte == '\n') {
            try out.append(allocator, byte);
            i += 1;
            continue;
        }
        if (byte == '\t') {
            try out.append(allocator, ' ');
            i += 1;
            continue;
        }
        if (byte < 0x20 or byte == 0x7f) {
            i += 1;
            continue;
        }
        const len = std.unicode.utf8ByteSequenceLength(byte) catch {
            if (out.items.len + 3 <= limit) try out.appendSlice(allocator, "\xef\xbf\xbd");
            i += 1;
            continue;
        };
        if (i + len > input.len) {
            if (out.items.len + 3 <= limit) try out.appendSlice(allocator, "\xef\xbf\xbd");
            break;
        }
        const cp = std.unicode.utf8Decode(input[i .. i + len]) catch {
            if (out.items.len + 3 <= limit) try out.appendSlice(allocator, "\xef\xbf\xbd");
            i += 1;
            continue;
        };
        if (cp >= 0x80 and cp <= 0x9f) {
            i += len;
            continue;
        }
        if (out.items.len + len > limit) break;
        try out.appendSlice(allocator, input[i .. i + len]);
        i += len;
    }
    return out.toOwnedSlice(allocator);
}

fn inputJson(a: std.mem.Allocator, kind: []const u8) ![]u8 {
    return std.json.Stringify.valueAlloc(a, .{ .type = "terminal/input", .kind = kind }, .{});
}
fn nonEmptyStringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = stringField(object, name) orelse return null;
    return if (value.len != 0 and std.mem.indexOfScalar(u8, value, 0) == null) value else null;
}
fn stringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |s| s,
        else => null,
    };
}

test "effect validation covers the whole native contract" {
    var valid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\",\"arg\"],\"completion\":\"done\",\"id\":\"1\"}", .{});
    defer valid.deinit();
    try validateEffect(valid.value);
    var invalid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\"],\"completion\":\"\",\"id\":\"\"}", .{});
    defer invalid.deinit();
    try std.testing.expectError(error.InvalidEffect, validateEffect(invalid.value));
    var bad_view = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"view/commit\",\"lines\":[{\"spans\":[{\"text\":\"bad\\n\"}]}]}", .{});
    defer bad_view.deinit();
    try std.testing.expectError(error.InvalidView, validateEffect(bad_view.value));
}

test "process output sanitizer removes terminal controls and repairs utf8" {
    const clean = try sanitizeProcessOutput(std.testing.allocator, "ok\tred\x1b[31m!\x1b[0m\r\n\x00\xc2\x85\xff!", 1024);
    defer std.testing.allocator.free(clean);
    try std.testing.expectEqualStrings("ok red!\n�!", clean);
    try std.testing.expect(std.unicode.utf8ValidateSlice(clean));
    try std.testing.expect(std.mem.indexOfScalar(u8, clean, 0x1b) == null);
}
