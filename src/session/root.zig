//! Non-reentrant FIFO owner for Lua transactions and fixed native effects.
const std = @import("std");
const auth = @import("misa_auth");
const file = @import("misa_file");
const http = @import("http.zig");
const operation = @import("operation.zig");
const lua = @import("misa_lua_runtime");
const state = @import("misa_state");
const terminal_module = @import("misa_terminal");
const process = @import("misa_process");

const buffered_records = @import("buffered_records.zig");
const input_event = @import("input_event.zig");
const native_effect = @import("native_effect.zig");
const timer = @import("timer.zig");
const max_operation_events_per_poll = 1;

pub const Session = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    runtime: *lua.Runtime,
    terminal: *terminal_module.Terminal,
    auth_store: ?*auth.Store,
    state_store: *state.Store,
    environ: *const std.process.Environ.Map,
    queue: std.ArrayList([]u8) = .empty,
    queue_head: usize = 0,
    input_events: std.ArrayList(terminal_module.Event) = .empty,
    input_head: usize = 0,
    running: bool = false,
    quit: bool = false,
    read_requested: bool = false,
    operations: operation.Owner,
    timers: timer.Collection,

    pub fn deinit(self: *Session) void {
        self.operations.deinit();
        self.timers.deinit();
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
        while (!self.quit and (self.queue_head < self.queue.items.len or self.read_requested or self.operations.isActive() or self.timers.count() != 0)) {
            try self.pollSources();
            // During a sustained stream, sample the terminal before dispatching
            // the next queued batch. The tty timeout provides backpressure while
            // keeping Ctrl-C latency and the session queue bounded.
            if (self.read_requested and self.operations.isActive()) try self.readTerminal();
            if (self.queue_head == self.queue.items.len) {
                if (self.read_requested) try self.readTerminal() else if (self.operations.isActive() or self.timers.count() != 0)
                    std.Io.sleep(self.io, .fromMilliseconds(10), .awake) catch {};
                continue;
            }
            const event = self.queue.items[self.queue_head];
            self.queue_head += 1;
            defer self.allocator.free(event);
            var transaction = self.runtime.dispatch(event) catch return error.LuaTransactionFailed;
            defer transaction.deinit();
            const view = transaction.view;
            var effects: std.ArrayList(native_effect.Effect) = .empty;
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
            if (!self.quit and self.queue_head == self.queue.items.len and self.read_requested) {
                try self.pollSources();
                if (self.queue_head == self.queue.items.len) try self.readTerminal();
            }
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

    fn execute(self: *Session, effect: native_effect.Effect) !void {
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
            .auth_command => |spec| try self.runAuth(spec),
            .state_load => |spec| try self.loadState(spec),
            .state_save => |spec| try self.state_store.save(spec.namespace, spec.data),
            .operation_cancel => |spec| try self.cancelOperation(spec.id),
            .timer_start => |spec| try self.timers.start(spec, std.Io.Timestamp.now(self.io, .awake).nanoseconds),
            .timer_stop => |spec| self.timers.stop(spec.id),
        }
    }

    fn pollSources(self: *Session) !void {
        if (try self.terminal.pollResize()) {
            self.runtime.setTerminalInfo(.{
                .interactive = self.terminal.interactive,
                .columns = self.terminal.dimensions.columns,
                .lines = self.terminal.dimensions.lines,
            });
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = "terminal/resize",
                .columns = self.terminal.dimensions.columns,
                .lines = self.terminal.dimensions.lines,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        }
        // A stream batch may fan out into many policy dispatches. Publish the
        // next native batch only after those events drain, preserving bounded
        // backpressure all the way through Session.queue.
        if (self.queue_head == self.queue.items.len) for (0..max_operation_events_per_poll) |_| {
            const item = try self.operations.pop() orelse break;
            errdefer self.allocator.free(item.json);
            try self.queue.append(self.allocator, item.json);
        };
        var due = self.timers.due(std.Io.Timestamp.now(self.io, .awake).nanoseconds);
        while (due.next()) |tick| {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = tick.completion,
                .id = tick.id,
                .tick = tick.tick,
                .elapsed_ms = tick.elapsed_ms,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
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

    fn loadState(self: *Session, spec: native_effect.StateLoad) !void {
        const loaded = try self.state_store.load(spec.namespace);
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = spec.completion,
            .namespace = spec.namespace,
            .found = loaded != null,
            .data = loaded orelse .null,
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
    }

    fn runProcess(self: *Session, spec: process.Spec) !void {
        if (spec.stdout_format == .json_lines_stream) return self.startProcess(spec);
        const result = try process.run(self.allocator, self.io, spec);
        defer result.deinit(self.allocator);
        if (spec.stdout_format == .json_lines and result.status == 0) {
            var records = try buffered_records.parseJsonLines(self.allocator, result.stdout);
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

    fn runAuth(self: *Session, spec: native_effect.AuthCommand) !void {
        const should_suspend = spec.action != .status;
        if (should_suspend) try self.terminal.suspendInput();
        const command_result = auth.command(self.allocator, self.io, self.environ, spec.action, spec.provider);
        // Restoring terminal ownership is part of the operation's success. Do
        // not hide a failed raw-mode/screen transition behind an auth result.
        if (should_suspend) try self.terminal.resumeInput();
        const result = command_result catch |err| {
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = false,
                .message = @errorName(err),
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
            return;
        };
        defer result.deinit(self.allocator);
        if (self.auth_store) |store| {
            const refreshed = auth.Store.init(self.allocator, self.io, self.environ) catch {
                const message = "credential updated; restart Misa before using it";
                const event = try std.json.Stringify.valueAlloc(self.allocator, .{ .type = spec.completion, .id = spec.id, .ok = true, .message = message }, .{});
                defer self.allocator.free(event);
                try self.enqueue(event);
                return;
            };
            store.deinit();
            store.* = refreshed;
        }
        const message = switch (spec.action) {
            .login => "logged in",
            .logout => "logged out",
            .status => if (result.logged_in) "logged in" else "logged out",
        };
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = spec.completion,
            .id = spec.id,
            .ok = true,
            .message = message,
            .provider = spec.provider,
            .logged_in = result.logged_in,
            .subscription_type = result.subscription_type,
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
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
        const text = try process.sanitizeOutput(self.allocator, result, 1024 * 1024);
        defer self.allocator.free(text);
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = spec.completion(),
            .id = spec.requestId(),
            .ok = true,
            .text = text,
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
    }

    fn decodeJson(self: *Session, spec: native_effect.JsonDecode) !void {
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
        if (spec.response_format == .sse_json_stream) return self.startHttp(spec);
        const result = http.run(self.allocator, self.io, self.auth_store, spec) catch |err| {
            try self.enqueueHttpError(spec, err);
            return;
        };
        defer result.deinit(self.allocator);
        const ok = result.status >= 200 and result.status < 300;
        if (spec.response_format != .text and ok) {
            var data = (switch (spec.response_format) {
                .json => std.json.parseFromSlice(std.json.Value, self.allocator, result.body, .{ .allocate = .alloc_always }),
                .sse_json => buffered_records.parseSseJson(self.allocator, result.body),
                .text, .sse_json_stream => unreachable,
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
            const body = try process.sanitizeOutput(self.allocator, result.body, 8 * 1024 * 1024);
            defer self.allocator.free(body);
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = spec.completion,
                .id = spec.id,
                .ok = ok,
                .status = result.status,
                .body = body,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        }
    }

    fn startProcess(self: *Session, source: process.Spec) !void {
        try self.enqueueStreamStart(source.completion, source.id);
        self.operations.startProcess(source) catch |err| {
            if (self.queue.pop()) |json| self.allocator.free(json);
            return err;
        };
        if (self.terminal.interactive) self.read_requested = true;
    }

    fn startHttp(self: *Session, source: http.Spec) !void {
        try self.enqueueStreamStart(source.completion, source.id);
        self.operations.startHttp(source, self.auth_store) catch |err| {
            if (self.queue.pop()) |json| self.allocator.free(json);
            return err;
        };
        if (self.terminal.interactive) self.read_requested = true;
    }

    fn cancelOperation(self: *Session, id: []const u8) !void {
        if (try self.operations.cancel(id)) |item| {
            errdefer self.allocator.free(item.json);
            try self.queue.append(self.allocator, item.json);
        }
    }

    fn enqueueStreamStart(self: *Session, completion: []const u8, id: []const u8) !void {
        const event = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = completion,
            .id = id,
            .phase = "start",
            .status = @as(i64, 0),
        }, .{});
        defer self.allocator.free(event);
        try self.enqueue(event);
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
        const json = try input_event.serialize(self.allocator, event);
        defer self.allocator.free(json);
        try self.enqueue(json);
    }
};
