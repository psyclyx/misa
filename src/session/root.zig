//! Non-reentrant FIFO owner for Lua transactions and fixed native effects.
const std = @import("std");
const auth = @import("misa_auth");
const file = @import("misa_file");
const http = @import("http.zig");
const lua = @import("misa_lua_runtime");
const terminal_module = @import("misa_terminal");
const process = @import("misa_process");

const JsonDecode = struct { source: []const u8, completion: []const u8, id: []const u8 };

const NativeEffect = union(enum) {
    dispatch: std.json.Value,
    terminal_read,
    view_commit: std.json.Value,
    app_quit,
    process_run: process.Spec,
    http_request: http.Spec,
    file: file.Spec,
    json_decode: JsonDecode,

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
        if (std.mem.eql(u8, kind, "http/request")) return .{ .http_request = try .parse(object) };
        if (std.mem.startsWith(u8, kind, "file/")) return .{ .file = try .parse(kind, object) };
        if (std.mem.eql(u8, kind, "json/decode")) return .{ .json_decode = .{
            .source = stringField(object, "source") orelse return error.InvalidEffect,
            .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
        } };
        return error.UnknownNativeEffect;
    }
};

pub const Session = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    runtime: *lua.Runtime,
    terminal: *terminal_module.Terminal,
    auth_store: ?*auth.Store,
    queue: std.ArrayList([]u8) = .empty,
    queue_head: usize = 0,
    input_events: std.ArrayList(terminal_module.Event) = .empty,
    input_head: usize = 0,
    running: bool = false,
    quit: bool = false,
    read_requested: bool = false,

    pub fn deinit(self: *Session) void {
        for (self.queue.items[self.queue_head..]) |item| self.allocator.free(item);
        self.queue.deinit(self.allocator);
        for (self.input_events.items[self.input_head..]) |event| event.deinit(self.allocator);
        self.input_events.deinit(self.allocator);
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
            // A read request is level-triggered. Decoded terminal events are
            // released one at a time so policy effects from Enter run before
            // bytes that followed it in the same OS read.
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
            .http_request => |spec| try self.runHttp(spec),
            .file => |spec| try self.runFile(spec),
            .json_decode => |spec| try self.decodeJson(spec),
        }
    }

    fn readTerminal(self: *Session) !void {
        self.read_requested = false;
        if (self.input_head == self.input_events.items.len) {
            self.input_events.clearRetainingCapacity();
            self.input_head = 0;
            try self.terminal.readEvents(&self.input_events);
        }
        if (self.input_head == self.input_events.items.len) {
            self.read_requested = true;
            return;
        }
        const event = self.input_events.items[self.input_head];
        self.input_head += 1;
        defer event.deinit(self.allocator);
        try self.enqueueInput(event);
        if (self.input_head == self.input_events.items.len) {
            self.input_events.clearRetainingCapacity();
            self.input_head = 0;
        }
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

    fn runFile(self: *Session, spec: file.Spec) !void {
        const result = file.run(self.allocator, self.io, spec) catch |err| {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion(),
                .id = spec.requestId(),
                .ok = false,
                .message = @errorName(err),
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
            return;
        };
        defer self.allocator.free(result);
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = spec.completion(),
            .id = spec.requestId(),
            .ok = true,
            .text = result,
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
    }

    fn decodeJson(self: *Session, spec: JsonDecode) !void {
        var parsed = std.json.parseFromSlice(std.json.Value, self.allocator, spec.source, .{ .allocate = .alloc_always }) catch {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = false,
                .message = "invalid JSON",
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
            return;
        };
        defer parsed.deinit();
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = spec.completion,
            .id = spec.id,
            .ok = true,
            .data = parsed.value,
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
    }

    fn runHttp(self: *Session, spec: http.Spec) !void {
        const store = self.auth_store orelse {
            try self.enqueueHttpError(spec, error.CredentialStoreUnavailable);
            return;
        };
        const result = http.run(self.allocator, self.io, store, spec) catch |err| {
            try self.enqueueHttpError(spec, err);
            return;
        };
        defer result.deinit(self.allocator);
        if (spec.response_format != .text and result.status >= 200 and result.status < 300) {
            var data = (switch (spec.response_format) {
                .json => std.json.parseFromSlice(std.json.Value, self.allocator, result.body, .{ .allocate = .alloc_always }),
                .sse_json => parseSseJson(self.allocator, result.body),
                .text => unreachable,
            }) catch {
                const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                    .type = spec.completion,
                    .id = spec.id,
                    .ok = false,
                    .status = result.status,
                    .body = "",
                    .message = "InvalidJsonResponse",
                }, .{});
                defer self.allocator.free(event);
                try self.enqueue(event);
                return;
            };
            defer data.deinit();
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = true,
                .status = result.status,
                .data = data.value,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        } else {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = result.status >= 200 and result.status < 300,
                .status = result.status,
                .body = result.body,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        }
    }

    fn enqueueHttpError(self: *Session, spec: http.Spec, err: anyerror) !void {
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = spec.completion,
            .id = spec.id,
            .ok = false,
            .status = @as(u16, 0),
            .body = "",
            .message = @errorName(err),
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
    }

    fn enqueueInput(self: *Session, event: terminal_module.Event) !void {
        const json = switch (event) {
            .text => |text| try std.json.Stringify.valueAlloc(self.allocator, .{ .type = "terminal/input", .kind = "text", .text = text }, .{}),
            .enter => try inputJson(self.allocator, "enter"),
            .backspace => try inputJson(self.allocator, "backspace"),
            .tab => try inputJson(self.allocator, "tab"),
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
fn parseSseJson(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(std.json.Value) {
    var document: std.ArrayList(u8) = .empty;
    defer document.deinit(allocator);
    try document.append(allocator, '[');
    var lines = std.mem.splitScalar(u8, source, '\n');
    var first = true;
    while (lines.next()) |raw| {
        const line = std.mem.trim(u8, raw, " \t\r");
        if (!std.mem.startsWith(u8, line, "data:")) continue;
        const data = std.mem.trimStart(u8, line[5..], " \t");
        if (data.len == 0 or std.mem.eql(u8, data, "[DONE]")) continue;
        if (!first) try document.append(allocator, ',');
        first = false;
        try document.appendSlice(allocator, data);
    }
    try document.append(allocator, ']');
    return std.json.parseFromSlice(std.json.Value, allocator, document.items, .{ .allocate = .alloc_always });
}

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

fn stringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |item| item,
        else => null,
    };
}

fn nonEmptyStringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const string = stringField(object, name) orelse return null;
    return if (string.len != 0 and std.mem.indexOfScalar(u8, string, 0) == null) string else null;
}

test "SSE data records become one owned array" {
    var parsed = try parseSseJson(std.testing.allocator, "event: update\ndata: {\"type\":\"delta\"}\n\ndata: [DONE]\n");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(usize, 1), parsed.value.array.items.len);
    try std.testing.expectEqualStrings("delta", parsed.value.array.items[0].object.get("type").?.string);
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
