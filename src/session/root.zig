//! Non-reentrant FIFO owner for Lua transactions and fixed native effects.
const std = @import("std");
const auth = @import("misa_auth");
const file = @import("misa_file");
const http = @import("misa_http");
const operation = @import("operation.zig");
const lua = @import("misa_lua_runtime");
const state = @import("misa_state");
const terminal_module = @import("misa_terminal");
const process = @import("misa_process");

const input_event = @import("input_event.zig");
const native_effect = @import("native_effect.zig");
const timer = @import("timer.zig");
const protected_input = @import("protected_input.zig");
const max_operation_events_per_poll = 1;

/// Bound synchronous dispatch fan-out without changing Lua execution or the
/// ordering of ordinary settled chains.
pub fn dispatchChainLimit(config: std.json.Value) !usize {
    if (config != .object) return 1024;
    const runtime = config.object.get("runtime") orelse return 1024;
    if (runtime != .object) return error.InvalidRuntimeConfig;
    const limit = runtime.object.get("max_dispatch_chain") orelse return 1024;
    if (limit != .integer or limit.integer < 1 or limit.integer > 1_000_000) return error.InvalidDispatchChainLimit;
    return @intCast(limit.integer);
}

/// A policy fault an interactive session reports and survives.
const Fault = enum { handler_error, effect_error, presentation_error };

/// The first line of a multi-line diagnostic, for a one-line notice.
fn firstLine(value: []const u8) []const u8 {
    return value[0 .. std.mem.indexOfScalar(u8, value, 0x0a) orelse value.len];
}

/// Name the type of a native effect that failed validation.
fn effectType(value: std.json.Value) []const u8 {
    if (value != .object) return "unknown";
    const kind = value.object.get("type") orelse return "unknown";
    return if (kind == .string) kind.string else "unknown";
}

pub const Session = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    runtime: *lua.Runtime,
    terminal: *terminal_module.Driver,
    interactive: bool,
    images_supported: bool,
    dimensions: terminal_module.Dimensions,
    environ: *const std.process.Environ.Map,
    queue: std.ArrayList([]u8) = .empty,
    queue_head: usize = 0,
    running: bool = false,
    quit: bool = false,
    read_requested: bool = false,
    dispatch_chain_limit: usize = 1024,
    operations: operation.Owner,
    timers: timer.Collection,
    pending_view: ?terminal_module.Driver.View = null,
    projection_dirty: bool = false,
    presentation_fault_reported: bool = false,
    protected: ?protected_input.Input = null,
    protected_wait: ?[]u8 = null,

    pub fn deinit(self: *Session) void {
        if (self.protected) |*input| input.deinit();
        if (self.protected_wait) |id| self.allocator.free(id);
        self.operations.deinit();
        self.timers.deinit();
        if (self.pending_view) |*view| view.deinit();
        for (self.queue.items[self.queue_head..]) |item| self.allocator.free(item);
        self.queue.deinit(self.allocator);
    }

    /// Seed app/start, then drain the queue. Dispatch effects only append; they never recurse.
    pub fn run(self: *Session) !void {
        std.debug.assert(!self.running);
        self.running = true;
        defer self.running = false;
        try self.enqueue("{\"type\":\"app/start\"}");
        var chain_events: usize = 0;
        while (self.hasWork()) {
            try self.terminal.checkError();
            if (self.queue_head == self.queue.items.len) {
                chain_events = 0;
                // A native source event and its synchronous dispatch effects
                // form one visible update. Settle that chain before showing a
                // frame or introducing another source batch. Polling only at
                // this boundary also prevents a busy stream/timer from keeping
                // the queue nonempty forever and starving presentation.
                try self.publishView();
                try self.pollSources();
                if (self.queue_head == self.queue.items.len) {
                    // Publishing the final view can exhaust the session. The
                    // terminal's flush acknowledgement handles that last frame.
                    if (!self.hasWork()) break;
                    try self.waitForSource();
                    continue;
                }
            }
            if (chain_events == self.dispatch_chain_limit) {
                var interrupted = try std.json.parseFromSlice(std.json.Value, self.allocator, self.queue.items[self.queue_head], .{ .allocate = .alloc_always });
                defer interrupted.deinit();
                const kind = if (interrupted.value == .object) interrupted.value.object.get("type") else null;
                const event_type = if (kind != null and kind.? == .string) kind.?.string else "unknown";
                // Already committed transactions/effects stay committed. Only
                // the remaining synchronous chain is discarded at this boundary.
                for (self.queue.items[self.queue_head..]) |queued| self.allocator.free(queued);
                self.queue.clearRetainingCapacity();
                self.queue_head = 0;
                if (!self.interactive) return error.DispatchChainLimitExceeded;
                self.read_requested = true;
                try self.publishView();
                try self.pollSources();
                const message = try std.fmt.allocPrint(self.allocator, "Stopped dispatch chain at '{s}': config.runtime.max_dispatch_chain exceeded; previously committed changes remain.", .{event_type});
                defer self.allocator.free(message);
                const notice = try std.json.Stringify.valueAlloc(self.allocator, .{
                    .type = "runtime/dispatch-limit",
                    .limit = self.dispatch_chain_limit,
                    .event_type = event_type,
                    .text = message,
                }, .{});
                defer self.allocator.free(notice);
                try self.enqueue(notice);
                chain_events = 0;
                continue;
            }
            chain_events += 1;
            const event = self.queue.items[self.queue_head];
            self.queue_head += 1;
            defer self.allocator.free(event);
            const clock: lua.ClockInfo = .{
                .wall_ms = std.Io.Timestamp.now(self.io, .real).toMilliseconds(),
                .monotonic_ms = std.Io.Timestamp.now(self.io, .awake).toMilliseconds(),
            };
            var transaction = self.runtime.dispatch(event, clock) catch {
                // A policy fault must not end an interactive session. Dispatch
                // already rolled the transaction back, so report it and keep
                // consuming events. A headless run keeps its exit status.
                if (!self.interactive) return error.LuaTransactionFailed;
                const name = try self.ownedEventName(event);
                defer self.allocator.free(name);
                try self.reportFault(.handler_error, name, firstLine(self.runtime.lastError()));
                // A reported fault still owns the terminal: keep reading so the
                // session stays interactive after the transaction was dropped.
                self.read_requested = true;
                continue;
            };
            defer transaction.deinit();
            errdefer self.runtime.rollbackTransaction();
            var effects: std.ArrayList(native_effect.Effect) = .empty;
            defer effects.deinit(self.allocator);
            var invalid: ?anyerror = null;
            var invalid_type: []const u8 = "unknown";
            for (transaction.effects) |value| {
                const parsed = native_effect.Effect.parse(value) catch |err| {
                    invalid = err;
                    invalid_type = effectType(value);
                    break;
                };
                try effects.append(self.allocator, parsed);
            }
            if (invalid) |failure| {
                self.runtime.rollbackTransaction();
                if (!self.interactive) return failure;
                const name = try self.ownedEventName(event);
                defer self.allocator.free(name);
                const detail = try std.fmt.allocPrint(self.allocator, "'{s}': {s}", .{ invalid_type, @errorName(failure) });
                defer self.allocator.free(detail);
                try self.reportFault(.effect_error, name, detail);
                self.read_requested = true;
                continue;
            }
            try self.runtime.commitTransaction();
            self.projection_dirty = true;
            for (effects.items) |effect| try self.execute(effect);
            self.compactQueue();
            // A read request is level-triggered. Decoded terminal events are
            // released one at a time so policy effects from Enter run before
            // bytes that followed it in the same OS read.
            // Sources are polled between settled dispatch chains. The current
            // chain drains before the next key is interpreted, so command effects
            // can install input ownership before trailing pasted bytes arrive.
        }
        try self.publishView();
        try self.terminal.flush();
    }

    fn hasWork(self: *const Session) bool {
        return (!self.quit or self.operations.isActive()) and
            (self.queue_head < self.queue.items.len or self.read_requested or
                self.operations.isActive() or self.timers.count() != 0 or self.pending_view != null or self.projection_dirty);
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
            .clipboard_write => |text| try self.terminal.copyToClipboard(text),
            .input_protected => |spec| {
                if (!self.operations.expectsInput(spec.id, spec.correlation)) return error.InteractionCorrelationMismatch;
                if (self.protected_wait) |id| {
                    if (!std.mem.eql(u8, id, spec.id)) return error.InteractionCorrelationMismatch;
                    self.allocator.free(id);
                    self.protected_wait = null;
                }
                if (self.protected) |*old| old.deinit();
                self.protected = null;
                self.protected = try .init(self.allocator, spec);
                self.read_requested = true;
            },
            .view_commit => |lines| try self.terminal.commit(lines),
            .app_quit => self.quit = true,
            .process_run => |spec| try self.startProcess(spec),
            .provider_process => |spec| try self.startProviderProcess(spec),
            .image => |spec| try self.operations.startImage(spec, self.environ),
            .syntax_highlight => |spec| try self.operations.startSyntax(spec),
            .http_request => |spec| try self.startHttp(spec),
            .file => |spec| try self.operations.startFile(spec),
            .json_decode => |spec| try self.decodeJson(spec),
            .auth_command => |spec| try self.startAuth(spec),
            .auth_respond => |spec| try self.operations.respond(spec.id, spec.correlation, spec.action, spec.value),
            .state_load => |spec| try self.operations.startStateLoad(spec.namespace, spec.completion, self.environ),
            .state_save => |spec| try self.operations.startStateSave(spec.namespace, spec.data, self.environ),
            .conversation_append => |spec| try self.operations.startConversation(spec, self.environ),
            .operation_cancel => |spec| try self.cancelOperation(spec.id),
            .operation_finish => |spec| try self.finishOperation(spec.id),
            .timer_start => |spec| try self.timers.start(spec, std.Io.Timestamp.now(self.io, .awake).nanoseconds),
            .timer_stop => |spec| self.timers.stop(spec.id),
        }
    }

    fn pollSources(self: *Session) !void {
        // Acquisition runs independently; interpretation releases only one key
        // after each settled chain, including installation of protected input.
        if (self.read_requested and self.protected_wait == null and self.queue_head == self.queue.items.len) {
            try self.terminal.enableInput();
            if (try self.terminal.popInput()) |input| try self.readTerminal(input);
        }
        if (try self.terminal.takeResize()) |dimensions| {
            self.dimensions = dimensions;
            self.runtime.setTerminalInfo(.{
                .interactive = self.interactive,
                .images = self.images_supported,
                .columns = terminal_module.usableColumns(dimensions.columns),
                .lines = dimensions.lines,
            });
            const event = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = "terminal/resize",
                .columns = dimensions.columns,
                .lines = dimensions.lines,
            }, .{});
            defer self.allocator.free(event);
            try self.enqueue(event);
        }
        // A stream batch may fan out into many policy dispatches. Publish the
        // next native batch only after those events drain, preserving bounded
        // backpressure all the way through Session.queue. Owner.pop itself is
        // round-robin, so this gate is bounded without starving another peer.
        if (self.queue_head == self.queue.items.len) for (0..max_operation_events_per_poll) |_| {
            const item = try self.operations.pop(std.Io.Timestamp.now(self.io, .awake).nanoseconds) orelse break;
            errdefer self.allocator.free(item.json);
            if (item.terminal_lease) |generation| {
                self.terminal.releaseHandoff(.{ .generation = generation }) catch |err| {
                    // Resume failures are operation failures, never fatal to the
                    // whole policy session.
                    self.allocator.free(item.json);
                    const event = try std.json.Stringify.valueAlloc(self.allocator, .{ .type = "auth/resume-failed", .id = "terminal-handoff", .ok = false, .message = @errorName(err) }, .{});
                    try self.queue.append(self.allocator, event);
                    continue;
                };
            }
            try self.queue.append(self.allocator, item.json);
        };
        if (self.protected) |*input| if (self.operations.find(input.spec.id) == null) {
            try self.discardBufferedInput();
            input.deinit();
            self.protected = null;
        };
        if (self.protected_wait) |id| if (self.operations.find(id) == null) {
            try self.discardBufferedInput();
            self.allocator.free(id);
            self.protected_wait = null;
            self.read_requested = true;
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

    fn waitForSource(self: *Session) !void {
        // Consume notifications before inspecting state, so a publication
        // between the state check and poll leaves a byte that wakes us.
        self.terminal.consumeWakeup();
        self.operations.consumeWakeup();
        try self.pollSources();
        if (self.queue_head < self.queue.items.len) return;
        try self.terminal.checkError();
        const now = std.Io.Timestamp.now(self.io, .awake).nanoseconds;
        var deadline: ?i96 = self.timers.nextDeadline();
        if (self.operations.nextDeadline()) |value| deadline = if (deadline) |old| @min(old, value) else value;
        const wait_ms: i32 = if (deadline) |value| @intCast(@min(@as(i96, std.math.maxInt(i32)), @divTrunc(@max(@as(i96, 0), value - now) + std.time.ns_per_ms - 1, std.time.ns_per_ms))) else -1;
        var fds = [_]std.posix.pollfd{
            .{ .fd = self.terminal.wakeupFd(), .events = std.posix.POLL.IN, .revents = 0 },
            .{ .fd = self.operations.wakeupFd(), .events = std.posix.POLL.IN, .revents = 0 },
        };
        _ = try std.posix.poll(&fds, wait_ms);
    }

    fn readTerminal(self: *Session, incoming: terminal_module.Driver.Input) !void {
        defer incoming.deinit(self.allocator);
        self.read_requested = false;
        const event = switch (incoming) {
            .key => |key| key,
            .action => |action| {
                if (self.protected == null) {
                    const json = try std.json.Stringify.valueAlloc(self.allocator, .{ .type = "ui/action", .action = action }, .{});
                    defer self.allocator.free(json);
                    try self.enqueue(json);
                }
                self.read_requested = true;
                return;
            },
            .hover => |target| {
                const json = try std.json.Stringify.valueAlloc(self.allocator, .{ .type = "ui/hover", .action = if (target.link) "" else target.value, .link = if (target.link) target.value else "" }, .{});
                defer self.allocator.free(json);
                try self.enqueue(json);
                self.read_requested = true;
                return;
            },
        };
        if (self.protected) |*input| {
            const result = input.accept(event);
            const json = try std.json.Stringify.valueAlloc(self.allocator, .{
                .type = input.spec.completion,
                .id = input.spec.id,
                .correlation = input.spec.correlation,
                .length = input.characters(),
                .too_long = input.too_long,
                .submitted = result == .submit,
                .cancelled = result == .cancel,
            }, .{});
            defer self.allocator.free(json);
            if (result == .submit) try self.operations.respond(input.spec.id, input.spec.correlation, "submit", input.text());
            try self.enqueue(json);
            if (result != .changed) {
                input.deinit();
                self.protected = null;
            }
            self.read_requested = true;
        } else try self.enqueueInput(event);
    }

    fn startAuth(self: *Session, spec: native_effect.AuthCommand) !void {
        const should_suspend = spec.action == .login and (spec.declaration.strategy == .cli_handoff or
            (spec.declaration.strategy == .api_key and !self.interactive));
        var lease: ?terminal_module.Terminal.HandoffLease = null;
        if (should_suspend) {
            self.read_requested = false;
            lease = try self.terminal.acquireHandoff();
        }
        // Wait for the protected prompt before releasing any buffered keys.
        if (spec.action == .login and spec.declaration.strategy == .api_key and self.interactive) {
            if (self.protected_wait != null or self.protected != null) return error.ProtectedInputAlreadyActive;
            self.protected_wait = try self.allocator.dupe(u8, spec.id);
        }
        self.operations.startAuth(spec.action, spec.declaration, spec.completion, spec.interaction, spec.id, self.environ, if (lease) |value| value.generation else null, self.interactive) catch |err| {
            if (lease) |value| self.terminal.releaseHandoff(value) catch {};
            return err;
        };
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

    fn startProcess(self: *Session, source: process.Spec) !void {
        if (source.stdout_format == .json_lines_stream) try self.enqueueStreamStart(source.completion, source.id);
        self.operations.startProcess(source) catch |err| {
            if (source.stdout_format == .json_lines_stream) if (self.queue.pop()) |json| self.allocator.free(json);
            return err;
        };
        if (self.interactive) self.read_requested = true;
    }

    fn startProviderProcess(self: *Session, source: process.Spec) !void {
        if (source.stdout_format == .json_lines_stream) try self.enqueueStreamStart(source.completion, source.id);
        self.operations.startProviderProcess(source, self.environ) catch |err| {
            if (source.stdout_format == .json_lines_stream) if (self.queue.pop()) |json| self.allocator.free(json);
            return err;
        };
        if (self.interactive) self.read_requested = true;
    }

    fn startHttp(self: *Session, source: http.Spec) !void {
        if (source.response_format == .sse_json_stream) try self.enqueueStreamStart(source.completion, source.id);
        self.operations.startHttp(source, self.environ) catch |err| {
            if (source.response_format == .sse_json_stream) if (self.queue.pop()) |json| self.allocator.free(json);
            return err;
        };
        if (self.interactive) self.read_requested = true;
    }

    fn cancelOperation(self: *Session, id: []const u8) !void {
        if (self.protected) |*input| if (std.mem.eql(u8, input.spec.id, id)) {
            try self.discardBufferedInput();
            input.deinit();
            self.protected = null;
        };
        if (try self.operations.cancel(id)) |item| {
            errdefer self.allocator.free(item.json);
            try self.queue.append(self.allocator, item.json);
        }
    }

    fn discardBufferedInput(self: *Session) !void {
        try self.terminal.discardInput();
    }

    fn finishOperation(self: *Session, id: []const u8) !void {
        if (try self.operations.finish(id)) |item| {
            errdefer self.allocator.free(item.json);
            try self.queue.append(self.allocator, item.json);
        }
    }

    /// Report a policy fault as an ordinary event. Interactive sessions
    /// carry on; a headless run reports the same fault by exiting.
    fn reportFault(self: *Session, kind: Fault, event_type: []const u8, detail: []const u8) !void {
        const message = try switch (kind) {
            .handler_error => std.fmt.allocPrint(self.allocator, "Handler failed for '{s}': {s} The transaction was rolled back.", .{ event_type, detail }),
            .effect_error => std.fmt.allocPrint(self.allocator, "Invalid native effect from '{s}': {s} The transaction was rolled back.", .{ event_type, detail }),
            .presentation_error => std.fmt.allocPrint(self.allocator, "Presentation failed: {s} The previous frame stays visible and the model is unchanged.", .{detail}),
        };
        defer self.allocator.free(message);
        const notice = try std.json.Stringify.valueAlloc(self.allocator, .{
            .type = switch (kind) {
                .handler_error => "runtime/handler-error",
                .effect_error => "runtime/effect-error",
                .presentation_error => "runtime/presentation-error",
            },
            .event_type = event_type,
            .text = message,
        }, .{});
        defer self.allocator.free(notice);
        try self.enqueue(notice);
    }

    /// Name a queued event for a fault notice. The caller frees the result.
    fn ownedEventName(self: *Session, json: []const u8) ![]u8 {
        var parsed = std.json.parseFromSlice(std.json.Value, self.allocator, json, .{}) catch return self.allocator.dupe(u8, "unknown");
        defer parsed.deinit();
        if (parsed.value != .object) return self.allocator.dupe(u8, "unknown");
        const kind = parsed.value.object.get("type") orelse return self.allocator.dupe(u8, "unknown");
        return self.allocator.dupe(u8, if (kind == .string) kind.string else "unknown");
    }

    fn publishView(self: *Session) !void {
        if (self.projection_dirty) {
            // A failed projection cannot undo committed model changes or
            // effects. Retain the last valid frame and try only after a later
            // transaction; otherwise a broken view would spin without input.
            self.projection_dirty = false;
            self.projectView() catch |err| {
                // Report the first fault of an episode: reporting every
                // attempt would keep the queue non-empty indefinitely.
                if (!self.presentation_fault_reported) {
                    self.presentation_fault_reported = true;
                    try self.reportFault(.presentation_error, "presentation", @errorName(err));
                }
                return;
            };
            self.presentation_fault_reported = false;
        }
        const view = self.pending_view orelse return;
        try self.terminal.publish(view);
        self.pending_view = null;
    }

    fn projectView(self: *Session) !void {
        var projection = try self.runtime.project(.{
            .wall_ms = std.Io.Timestamp.now(self.io, .real).toMilliseconds(),
            .monotonic_ms = std.Io.Timestamp.now(self.io, .awake).toMilliseconds(),
        });
        defer projection.deinit();
        errdefer self.runtime.rollbackProjection();
        if (projection.value != .null) terminal_module.validateView(self.allocator, projection.value, self.dimensions, self.images_supported) catch |err| {
            self.runtime.reportProjectionError(err);
            return err;
        };
        try self.runtime.commitProjection();
        if (projection.value != .null) {
            if (self.pending_view) |*old| old.deinit();
            self.pending_view = .{ .arena = projection.arena, .value = projection.value, .dimensions = self.dimensions };
            projection.arena = .init(self.allocator);
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

    fn enqueueInput(self: *Session, event: terminal_module.Event) !void {
        // Click actions were resolved on the terminal owner against the frame
        // visible when the bytes were acquired. Unmapped clicks are inert.
        if (event == .mouse) {
            self.read_requested = true;
            return;
        }
        const json = try input_event.serialize(self.allocator, event);
        defer self.allocator.free(json);
        try self.enqueue(json);
    }
};
