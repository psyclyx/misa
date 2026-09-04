//! Non-reentrant FIFO owner for Lua transactions and fixed native effects.
const std = @import("std");
const lua = @import("misa_lua_runtime");
const terminal_module = @import("misa_terminal");
const process = @import("process.zig");

const NativeEffect = union(enum) {
    dispatch: std.json.Value,
    terminal_read,
    view_commit: std.json.Value,
    app_quit,
    process_run: process.Spec,

    fn parse(value: std.json.Value) !NativeEffect {
        const object = switch (value) {
            .object => |item| item,
            else => return error.InvalidEffect,
        };
        const kind = nonEmptyStringField(object, "type") orelse return error.InvalidEffect;
        if (std.mem.eql(u8, kind, "dispatch")) {
            const event = object.get("event") orelse return error.InvalidEffect;
            const event_object = switch (event) {
                .object => |item| item,
                else => return error.InvalidEffect,
            };
            _ = nonEmptyStringField(event_object, "type") orelse return error.InvalidEffect;
            return .{ .dispatch = event };
        }
        if (std.mem.eql(u8, kind, "terminal/read")) return .terminal_read;
        if (std.mem.eql(u8, kind, "view/commit")) {
            const lines = object.get("lines") orelse return error.InvalidEffect;
            try terminal_module.validateLines(lines);
            return .{ .view_commit = lines };
        }
        if (std.mem.eql(u8, kind, "app/quit")) return .app_quit;
        if (std.mem.eql(u8, kind, "process/run")) return .{ .process_run = try .parse(object) };
        return error.UnknownNativeEffect;
    }
};

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
            var effects: std.ArrayList(NativeEffect) = .empty;
            defer effects.deinit(self.allocator);
            for (transaction.effects) |effect| try effects.append(self.allocator, try .parse(effect));
            // Validate everything, then make the pending semantic view visible
            // before committing policy state. Validation/presentation failures
            // leave canonical Lua db unchanged. After commit, effect I/O is
            // fatal on failure and cannot in general be rolled back.
            var presentation: ?terminal_module.PreparedPresentation = if (view != .null)
                try self.terminal.preparePresentation(view)
            else
                null;
            defer if (presentation) |*prepared| prepared.deinit(self.allocator);
            if (presentation) |*prepared| try self.terminal.present(prepared);
            try self.runtime.commitTransaction();
            for (effects.items) |effect| try self.execute(effect);
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

    fn execute(self: *Session, effect: NativeEffect) !void {
        switch (effect) {
            .dispatch => |event| {
                const json = try std.json.Stringify.valueAlloc(self.allocator, event, .{});
                defer self.allocator.free(json);
                try self.enqueue(json);
            },
            .terminal_read => self.read_requested = true,
            .view_commit => |lines| try self.terminal.commit(lines),
            .app_quit => self.quit = true,
            .process_run => |spec| try self.runProcess(spec),
        }
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

    fn runProcess(self: *Session, spec: process.Spec) !void {
        const result = try process.run(self.allocator, self.io, spec);
        defer result.deinit(self.allocator);
        if (spec.stdout_format == .json_lines and result.status == 0) {
            var records = try parseJsonLines(self.allocator, result.stdout);
            defer records.deinit();
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = true,
                .records = records.value,
                .stderr = result.stderr,
                .status = result.status,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        } else {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = result.status == 0,
                .stdout = result.stdout,
                .stderr = result.stderr,
                .status = result.status,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        }
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
fn parseJsonLines(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(std.json.Value) {
    var document: std.ArrayList(u8) = .empty;
    defer document.deinit(allocator);
    try document.append(allocator, '[');
    var lines = std.mem.splitScalar(u8, source, '\n');
    var first = true;
    while (lines.next()) |raw| {
        const line = std.mem.trim(u8, raw, " \t\r");
        if (line.len == 0) continue;
        if (!first) try document.append(allocator, ',');
        first = false;
        try document.appendSlice(allocator, line);
    }
    try document.append(allocator, ']');
    return std.json.parseFromSlice(std.json.Value, allocator, document.items, .{ .allocate = .alloc_always });
}

fn inputJson(allocator: std.mem.Allocator, kind: []const u8) ![]u8 {
    return std.json.Stringify.valueAlloc(allocator, .{ .type = "terminal/input", .kind = kind }, .{});
}

fn nonEmptyStringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    const string = switch (value) {
        .string => |item| item,
        else => return null,
    };
    return if (string.len != 0 and std.mem.indexOfScalar(u8, string, 0) == null) string else null;
}

test "JSON lines become one owned array" {
    var parsed = try parseJsonLines(std.testing.allocator, "{\"type\":\"start\"}\n\n{\"type\":\"result\",\"text\":\"ok\"}\n");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(usize, 2), parsed.value.array.items.len);
    try std.testing.expectEqualStrings("ok", parsed.value.array.items[1].object.get("text").?.string);
}

test "effect validation covers the whole native contract" {
    var valid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\",\"arg\"],\"completion\":\"done\",\"id\":\"1\"}", .{});
    defer valid.deinit();
    _ = try NativeEffect.parse(valid.value);
    var invalid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\"],\"completion\":\"\",\"id\":\"\"}", .{});
    defer invalid.deinit();
    try std.testing.expectError(error.InvalidEffect, NativeEffect.parse(invalid.value));
    var bad_view = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"view/commit\",\"lines\":[{\"spans\":[{\"text\":\"bad\\n\"}]}]}", .{});
    defer bad_view.deinit();
    try std.testing.expectError(error.InvalidView, NativeEffect.parse(bad_view.value));
}
