//! One native operation's owned inputs, worker lifetime, interaction, and result.
const std = @import("std");
const auth = @import("misa_auth");
const file = @import("misa_file");
const image = @import("misa_image");
const process = @import("misa_process");
const state = @import("misa_state");
const http = @import("../http.zig");
const buffered_records = @import("../buffered_records.zig");
const channel_module = @import("channel.zig");
const result_json = @import("result_json.zig");

const AuthSpec = struct { action: auth.Action, declaration: auth.Declaration, completion: []const u8, interaction: []const u8, id: []const u8, environ: *const std.process.Environ.Map, terminal_lease: ?u64, managed_input: bool };
const StateSpec = struct { namespace: []const u8, completion: []const u8, id: []const u8, data: ?std.json.Value, environ: *const std.process.Environ.Map };
const HttpSpec = struct { spec: http.Spec, environ: *const std.process.Environ.Map };
const ImageSpec = struct { spec: image.Spec, environ: *const std.process.Environ.Map };
const Kind = union(enum) { image: ImageSpec, http: HttpSpec, process: process.Spec, file: file.Spec, state_load: StateSpec, state_save: StateSpec, auth: AuthSpec };

pub const Task = struct {
    pub const max_records_per_event = 32;

    owner_allocator: std.mem.Allocator,
    arena: std.heap.ArenaAllocator,
    io: std.Io,
    kind: Kind,
    completion: []const u8 = "",
    id: []const u8 = "",
    fallback: ?[]u8 = null,
    result: result_json.Outcome = .{},
    done: std.atomic.Value(bool) = .init(false),
    group: std.Io.Group = .init,
    records: channel_module.Channel,
    response_mutex: std.Io.Mutex = .init,
    response_ready: std.Io.Condition = .init,
    response_correlation: ?[]u8 = null,
    response_action: ?[]u8 = null,
    response_value: ?[]u8 = null,
    response_expected: ?[]const u8 = null,
    started_ns: i96 = 0,
    timeouts: http.Spec.Timeouts = .{},
    timeout_kind: enum { http, process, none } = .http,

    fn init(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup) Task {
        return .{ .owner_allocator = owner_allocator, .arena = .init(owner_allocator), .io = io, .kind = undefined, .records = .init(owner_allocator, io, wakeup) };
    }

    fn create(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup) !*Task {
        const task = try owner_allocator.create(Task);
        task.* = .init(owner_allocator, io, wakeup);
        return task;
    }

    pub fn createProcess(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: process.Spec) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const argv = try a.alloc(std.json.Value, source.argv.len);
        for (source.argv, argv) |arg, *copy| copy.* = .{ .string = try a.dupe(u8, arg.string) };
        const spec: process.Spec = .{
            .argv = argv,
            .completion = try a.dupe(u8, source.completion),
            .id = try a.dupe(u8, source.id),
            .stdout_format = source.stdout_format,
            .stdin = if (source.stdin) |v| try a.dupe(u8, v) else null,
            .stdin_json = if (source.stdin_json) |v| try cloneJson(a, v) else null,
            .timeouts = source.timeouts,
        };
        task.kind = .{ .process = spec };
        try task.prepare(spec.completion, spec.id, .{ .first_byte_ms = spec.timeouts.startup_ms, .idle_ms = spec.timeouts.idle_ms, .overall_ms = spec.timeouts.overall_ms });
        task.timeout_kind = .process;
        return task;
    }

    pub fn createHttp(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: http.Spec, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const headers = try a.alloc(std.json.Value, source.headers.len);
        for (source.headers, headers) |value, *copy| copy.* = try cloneJson(a, value);
        const spec: http.Spec = .{
            .url = try a.dupe(u8, source.url),
            .method = source.method,
            .body = if (source.body) |v| try a.dupe(u8, v) else null,
            .json = if (source.json) |v| try cloneJson(a, v) else null,
            .headers = headers,
            .credential = if (source.credential) |credential| .{
                .id = try a.dupe(u8, credential.id),
                .header = try a.dupe(u8, credential.header),
                .prefix = try a.dupe(u8, credential.prefix),
                .metadata_field = if (credential.metadata_field) |v| try a.dupe(u8, v) else null,
                .metadata_header = if (credential.metadata_header) |v| try a.dupe(u8, v) else null,
            } else null,
            .completion = try a.dupe(u8, source.completion),
            .id = try a.dupe(u8, source.id),
            .response_format = source.response_format,
            .timeouts = source.timeouts,
        };
        task.kind = .{ .http = .{ .spec = spec, .environ = environ } };
        try task.prepare(spec.completion, spec.id, spec.timeouts);
        return task;
    }

    pub fn createFile(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: file.Spec) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const spec = try cloneFile(task.arena.allocator(), source);
        task.kind = .{ .file = spec };
        try task.prepare(spec.completion(), spec.requestId(), .{});
        return task;
    }

    pub fn createImage(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: image.Spec, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const argv = try a.alloc(std.json.Value, source.argv.len);
        for (source.argv, argv) |arg, *copy| copy.* = .{ .string = try a.dupe(u8, arg.string) };
        const spec: image.Spec = .{
            .path = if (source.path) |path| try a.dupe(u8, path) else null,
            .argv = argv,
            .id = try a.dupe(u8, source.id),
            .completion = try a.dupe(u8, source.completion),
        };
        task.kind = .{ .image = .{ .spec = spec, .environ = environ } };
        try task.prepare(spec.completion, spec.id, .{ .first_byte_ms = 10_000, .idle_ms = 10_000, .overall_ms = 15_000 });
        return task;
    }

    pub fn createState(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, namespace: []const u8, completion: []const u8, serial: u64, data: ?std.json.Value, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: StateSpec = .{ .namespace = try a.dupe(u8, namespace), .completion = try a.dupe(u8, completion), .id = try std.fmt.allocPrint(a, "state:{s}:{d}", .{ namespace, serial }), .data = if (data) |value| try cloneJson(a, value) else null, .environ = environ };
        task.kind = if (data == null) .{ .state_load = spec } else .{ .state_save = spec };
        try task.prepare(spec.completion, spec.id, .{});
        return task;
    }

    pub fn createAuth(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, action: auth.Action, declaration: auth.Declaration, completion: []const u8, interaction: []const u8, id: []const u8, environ: *const std.process.Environ.Map, terminal_lease: ?u64, managed_input: bool) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: AuthSpec = .{ .action = action, .declaration = try cloneDeclaration(a, declaration), .completion = try a.dupe(u8, completion), .interaction = try a.dupe(u8, interaction), .id = try a.dupe(u8, id), .environ = environ, .terminal_lease = terminal_lease, .managed_input = managed_input };
        task.kind = .{ .auth = spec };
        try task.prepare(spec.completion, spec.id, .{ .first_byte_ms = 900_000, .idle_ms = 900_000, .overall_ms = 900_000 });
        return task;
    }

    pub fn destroy(self: *Task) void {
        if (self.response_value) |bytes| std.crypto.secureZero(u8, bytes);
        self.records.discard();
        if (self.fallback) |json| self.owner_allocator.free(json);
        self.arena.deinit();
        self.owner_allocator.destroy(self);
    }

    pub fn start(self: *Task) void {
        self.group.async(self.io, worker, .{self});
    }

    pub fn cancel(self: *Task) void {
        self.group.cancel(self.io);
    }

    pub fn await(self: *Task) !void {
        self.group.await(self.io) catch |err| if (err != error.Canceled) return err;
    }

    pub fn isDone(self: *const Task) bool {
        return self.done.load(.acquire);
    }

    pub fn recordCount(self: *Task) usize {
        return self.records.count();
    }

    pub fn timeoutName(self: *Task, now: i96) ?[]const u8 {
        if (self.done.load(.acquire)) return null;
        const elapsed_ms = @divTrunc(now - self.started_ns, std.time.ns_per_ms);
        if (elapsed_ms >= self.timeouts.overall_ms) return "OverallTimeout";
        if (!self.records.saw_activity.load(.acquire)) {
            if (self.timeout_kind == .process) {
                if (elapsed_ms >= self.timeouts.first_byte_ms) return "StartupTimeout";
            } else if (elapsed_ms >= self.timeouts.first_byte_ms) return "FirstByteTimeout";
        } else {
            const idle_ms = @divTrunc(now - @as(i96, self.records.activity_ns.load(.acquire)), std.time.ns_per_ms);
            if (idle_ms >= self.timeouts.idle_ms) return "IdleTimeout";
        }
        return null;
    }

    pub fn nextDeadline(self: *const Task) i96 {
        const overall = self.started_ns + @as(i96, self.timeouts.overall_ms) * std.time.ns_per_ms;
        const phase = if (self.records.saw_activity.load(.acquire))
            @as(i96, self.records.activity_ns.load(.acquire)) + @as(i96, self.timeouts.idle_ms) * std.time.ns_per_ms
        else
            self.started_ns + @as(i96, self.timeouts.first_byte_ms) * std.time.ns_per_ms;
        return @min(overall, phase);
    }

    pub fn respond(self: *Task, correlation: []const u8, action: []const u8, value: []const u8) !void {
        try self.response_mutex.lock(self.io);
        defer self.response_mutex.unlock(self.io);
        if (self.response_value != null) return error.InteractionAlreadyAnswered;
        if (self.response_expected == null or !std.mem.eql(u8, self.response_expected.?, correlation)) return error.InteractionCorrelationMismatch;
        self.response_correlation = try self.arena.allocator().dupe(u8, correlation);
        self.response_action = try self.arena.allocator().dupe(u8, action);
        self.response_value = try self.arena.allocator().dupe(u8, value);
        self.response_ready.signal(self.io);
    }

    pub fn expectsInput(self: *Task, correlation: []const u8) bool {
        self.response_mutex.lockUncancelable(self.io);
        defer self.response_mutex.unlock(self.io);
        return self.response_value == null and self.response_expected != null and std.mem.eql(u8, self.response_expected.?, correlation);
    }

    pub fn popRecord(self: *Task) !?result_json.Item {
        var records: [max_records_per_event][]u8 = undefined;
        const drained = self.records.drain(&records);
        if (drained.event) |json| {
            defer self.owner_allocator.free(json);
            return .{ .json = try self.owner_allocator.dupe(u8, json) };
        }
        if (drained.count == 0 and !drained.terminal) return null;
        defer for (records[0..drained.count]) |json| self.owner_allocator.free(json);
        return .{ .json = try result_json.data(self.owner_allocator, self.completion, self.id, records[0..drained.count], drained.terminal) };
    }

    pub fn discardRecords(self: *Task) void {
        self.records.discard();
    }

    pub fn completionItem(self: *Task) !result_json.Item {
        const json = try result_json.outcome(self.owner_allocator, self.completion, self.id, self.resultKind(), self.result);
        self.owner_allocator.free(self.fallback.?);
        self.fallback = null;
        return .{ .json = json, .terminal = true, .terminal_lease = self.terminalLease() };
    }

    pub fn forcedItem(self: *Task, ok: bool, message: ?[]const u8) !result_json.Item {
        const json = try result_json.terminal(self.owner_allocator, self.completion, self.id, ok, 0, "", message);
        self.owner_allocator.free(self.fallback.?);
        self.fallback = null;
        return .{ .json = json, .terminal = true, .terminal_lease = self.terminalLease() };
    }

    pub fn takeFallback(self: *Task) result_json.Item {
        const json = self.fallback.?;
        self.fallback = null;
        return .{ .json = json, .terminal = true, .terminal_lease = self.terminalLease() };
    }

    fn prepare(self: *Task, completion: []const u8, id: []const u8, timeouts: http.Spec.Timeouts) !void {
        self.completion = completion;
        self.id = id;
        self.timeouts = timeouts;
        self.started_ns = std.Io.Timestamp.now(self.io, .awake).nanoseconds;
        self.records.resetActivity(self.started_ns);
        self.fallback = try result_json.terminal(self.owner_allocator, completion, id, false, 0, "", "OperationFailed");
    }

    fn terminalLease(self: *const Task) ?u64 {
        return switch (self.kind) {
            .auth => |spec| spec.terminal_lease,
            else => null,
        };
    }

    fn resultKind(self: *const Task) result_json.Kind {
        return switch (self.kind) {
            .auth => |spec| .{ .auth = .{ .action = spec.action, .declaration = spec.declaration } },
            .http => |request| if (request.spec.response_format == .sse_json_stream) .http_stream else .ordinary,
            .process => |spec| if (spec.stdout_format == .json_lines_stream) .process_stream else .ordinary,
            .file => .file,
            .image => .ordinary,
            .state_load => |spec| .{ .state_load = spec.namespace },
            .state_save => .state_save,
        };
    }

    fn emitHttpRecord(context: *anyopaque, raw: ?[]const u8) anyerror!void {
        const self: *Task = @ptrCast(@alignCast(context));
        if (raw == null) {
            try self.records.push(.terminal);
            return error.StreamFinished;
        }
        var parsed = try std.json.parseFromSlice(std.json.Value, self.owner_allocator, raw.?, .{});
        defer parsed.deinit();
        const owned = try self.owner_allocator.dupe(u8, raw.?);
        errdefer self.owner_allocator.free(owned);
        try self.records.push(.{ .data = owned });
    }

    fn emitProcessRecord(context: *anyopaque, raw: []const u8) anyerror!void {
        return emitHttpRecord(context, raw);
    }

    fn noteActivityOpaque(context: *anyopaque) void {
        const self: *Task = @ptrCast(@alignCast(context));
        self.records.noteActivity();
    }

    fn emitInteraction(context: *anyopaque, prompt: auth.oauth.Prompt) anyerror!void {
        const self: *Task = @ptrCast(@alignCast(context));
        if (prompt.input) {
            try self.response_mutex.lock(self.io);
            defer self.response_mutex.unlock(self.io);
            self.response_expected = try self.arena.allocator().dupe(u8, prompt.correlation);
        }
        const json = try std.json.Stringify.valueAlloc(self.owner_allocator, .{ .type = self.kind.auth.interaction, .id = self.id, .phase = "interaction", .correlation = prompt.correlation, .kind = prompt.kind, .title = prompt.title, .message = prompt.message, .url = prompt.url, .code = prompt.code, .progress = prompt.progress, .cancellable = prompt.cancellable, .input = prompt.input, .protected = prompt.protected, .hints = prompt.hints, .actions = if (prompt.input) &.{.{ .id = "submit", .label = "enter submit", .primary = true }} else &.{} }, .{});
        errdefer self.owner_allocator.free(json);
        try self.records.push(.{ .event = json });
    }

    fn awaitInteraction(context: *anyopaque, correlation: []const u8) anyerror![]const u8 {
        const self: *Task = @ptrCast(@alignCast(context));
        try self.response_mutex.lock(self.io);
        defer self.response_mutex.unlock(self.io);
        while (self.response_value == null) try self.response_ready.wait(self.io, &self.response_mutex);
        if (!std.mem.eql(u8, self.response_correlation.?, correlation)) return error.InteractionCorrelationMismatch;
        return self.response_value.?;
    }

    fn worker(self: *Task) std.Io.Cancelable!void {
        defer {
            self.done.store(true, .release);
            self.records.wakeup.notify();
        }
        switch (self.kind) {
            .image => |request| {
                const value = image.run(self.arena.allocator(), self.io, request.spec, request.environ) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .data = value, .message = null };
            },
            .http => |request| if (request.spec.response_format == .sse_json_stream) {
                var store = if (request.spec.credential != null) auth.Store.init(self.arena.allocator(), self.io, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                } else null;
                defer if (store) |*value| value.deinit();
                if (request.spec.credential) |credential| auth.validateCredentialOrigin(credential.id, request.spec.url, if (store) |*value| value else null, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                const run_result = http.runSse(self.arena.allocator(), self.io, if (store) |*value| value else null, request.spec, .{ .context = self, .emit = emitHttpRecord, .activity = noteActivityOpaque }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = run_result.status >= 200 and run_result.status < 300 and run_result.failure == null, .status = run_result.status, .body = run_result.error_body, .message = if (run_result.failure) |failure| @errorName(failure) else null };
            } else {
                var store = if (request.spec.credential != null) auth.Store.init(self.arena.allocator(), self.io, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                } else null;
                defer if (store) |*value| value.deinit();
                if (request.spec.credential) |credential| auth.validateCredentialOrigin(credential.id, request.spec.url, if (store) |*value| value else null, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                const run_result = http.run(self.arena.allocator(), self.io, if (store) |*value| value else null, request.spec, .{ .context = self, .note = noteActivityOpaque }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.records.noteActivity();
                const ok = run_result.status >= 200 and run_result.status < 300;
                if (ok and request.spec.response_format != .text) {
                    const parsed = (switch (request.spec.response_format) {
                        .json => std.json.parseFromSlice(std.json.Value, self.arena.allocator(), run_result.body, .{ .allocate = .alloc_always }),
                        .sse_json => buffered_records.parseSseJson(self.arena.allocator(), run_result.body),
                        else => unreachable,
                    }) catch {
                        self.result = .{ .status = run_result.status, .message = "InvalidJsonResponse" };
                        return;
                    };
                    self.result = .{ .ok = true, .status = run_result.status, .message = null, .data = parsed.value };
                } else self.result = .{ .ok = ok, .status = run_result.status, .body = process.sanitizeOutput(self.arena.allocator(), run_result.body, 8 * 1024 * 1024) catch "", .message = null };
            },
            .process => |spec| if (spec.stdout_format == .json_lines_stream) {
                const run_result = process.runJsonLines(self.arena.allocator(), self.io, spec, .{ .context = self, .emit = emitProcessRecord, .activity = noteActivityOpaque }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .status = -1, .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = run_result.status == 0, .status = run_result.status, .body = run_result.stderr, .message = null };
            } else {
                const run_result = process.runWithActivity(self.arena.allocator(), self.io, spec, .{ .context = self, .note = noteActivityOpaque }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .status = -1, .message = @errorName(err) };
                    return;
                };
                self.records.noteActivity();
                if (spec.stdout_format == .json_lines and run_result.status == 0) {
                    const parsed = buffered_records.parseJsonLines(self.arena.allocator(), run_result.stdout) catch {
                        self.result = .{ .status = run_result.status, .message = "InvalidJsonResponse" };
                        return;
                    };
                    self.result = .{ .ok = true, .status = run_result.status, .body = run_result.stderr, .message = null, .data = parsed.value };
                } else self.result = .{ .ok = run_result.status == 0, .status = run_result.status, .body = run_result.stdout, .message = run_result.stderr };
            },
            .file => |spec| {
                const run_result = file.run(self.arena.allocator(), self.io, spec) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .body = process.sanitizeOutput(self.arena.allocator(), run_result, 1024 * 1024) catch "", .message = null };
            },
            .state_load => |spec| {
                var store = state.Store.init(self.arena.allocator(), self.io, spec.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                defer store.deinit();
                const loaded = store.load(spec.namespace) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .data = if (loaded) |value| cloneJson(self.arena.allocator(), value) catch null else null, .message = null };
            },
            .state_save => |spec| {
                var store = state.Store.init(self.arena.allocator(), self.io, spec.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                defer store.deinit();
                store.save(spec.namespace, spec.data.?) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .message = null };
            },
            .auth => |spec| {
                const command_result = auth.command(self.arena.allocator(), self.io, spec.environ, spec.action, spec.declaration, .{ .context = self, .emitFn = emitInteraction, .inputFn = awaitInteraction, .protected_input = spec.managed_input }) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .message = null, .logged_in = command_result.logged_in, .subscription_type = command_result.subscription_type };
            },
        }
    }
};

fn cloneDeclaration(a: std.mem.Allocator, source: auth.Declaration) !auth.Declaration {
    return .{ .provider = try a.dupe(u8, source.provider), .strategy = source.strategy, .profile_id = if (source.profile_id) |value| try a.dupe(u8, value) else null, .authorization_url = if (source.authorization_url) |value| try a.dupe(u8, value) else null, .token_url = if (source.token_url) |value| try a.dupe(u8, value) else null, .api_base = if (source.api_base) |value| try a.dupe(u8, value) else null };
}

fn cloneJson(a: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    const encoded = try std.json.Stringify.valueAlloc(a, value, .{});
    const parsed = try std.json.parseFromSlice(std.json.Value, a, encoded, .{ .allocate = .alloc_always });
    return parsed.value;
}

fn cloneFile(a: std.mem.Allocator, source: file.Spec) !file.Spec {
    return switch (source) {
        .read => |value| .{ .read = .{ .path = try a.dupe(u8, value.path), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
        .list => |value| .{ .list = .{ .path = try a.dupe(u8, value.path), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
        .write => |value| .{ .write = .{ .path = try a.dupe(u8, value.path), .content = try a.dupe(u8, value.content), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
        .edit => |value| .{ .edit = .{ .path = try a.dupe(u8, value.path), .old = try a.dupe(u8, value.old), .new = try a.dupe(u8, value.new), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
    };
}

test "task publishes correlated auth interaction events" {
    const wakeup = try channel_module.Wakeup.init();
    defer wakeup.deinit();
    const task = try Task.createAuth(std.testing.allocator, std.testing.io, wakeup, .login, .{ .provider = "openai-codex", .strategy = .device_oauth, .profile_id = "default", .authorization_url = "https://auth.openai.com/api/accounts/deviceauth/usercode", .token_url = "https://auth.openai.com/oauth/token" }, "auth/complete", "auth/interaction", "login:openai", undefined, null, true);
    defer task.destroy();
    try Task.emitInteraction(task, .{ .correlation = "device", .title = "Authorize device", .message = "Waiting", .url = "https://example.test", .code = "ABCD", .progress = "Polling" });
    const item = (try task.popRecord()).?;
    defer std.testing.allocator.free(item.json);
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, item.json, .{});
    defer parsed.deinit();
    try std.testing.expectEqualStrings("auth/interaction", parsed.value.object.get("type").?.string);
    try std.testing.expectEqualStrings("device", parsed.value.object.get("correlation").?.string);
    try std.testing.expectEqualStrings("ABCD", parsed.value.object.get("code").?.string);
}

test "task timeout distinguishes first byte, idle, and overall" {
    const wakeup = try channel_module.Wakeup.init();
    defer wakeup.deinit();
    const argv = [_]std.json.Value{.{ .string = "/bin/true" }};
    const task = try Task.createProcess(std.testing.allocator, std.testing.io, wakeup, .{ .argv = &argv, .completion = "done", .id = "timeout", .stdout_format = .text, .stdin = null, .stdin_json = null, .timeouts = .{ .startup_ms = 20, .idle_ms = 30, .overall_ms = 200 } });
    defer task.destroy();
    try std.testing.expectEqualStrings("StartupTimeout", task.timeoutName(task.started_ns + 25 * std.time.ns_per_ms).?);
    task.records.saw_activity.store(true, .release);
    task.records.activity_ns.store(@intCast(task.started_ns), .release);
    try std.testing.expectEqualStrings("IdleTimeout", task.timeoutName(task.started_ns + 35 * std.time.ns_per_ms).?);
    try std.testing.expectEqualStrings("OverallTimeout", task.timeoutName(task.started_ns + 250 * std.time.ns_per_ms).?);
}

test "task coalesces 32 ordered records and flushes terminal markers" {
    const wakeup = try channel_module.Wakeup.init();
    defer wakeup.deinit();
    const argv = [_]std.json.Value{.{ .string = "/bin/true" }};
    const task = try Task.createProcess(std.testing.allocator, std.testing.io, wakeup, .{ .argv = &argv, .completion = "stream/done", .id = "quoted-\"id", .stdout_format = .json_lines_stream, .stdin = null, .stdin_json = null });
    defer task.destroy();
    for (0..Task.max_records_per_event) |index| try task.records.push(.{ .data = try std.fmt.allocPrint(std.testing.allocator, "{{\"index\":{d}}}", .{index}) });
    const first = (try task.popRecord()).?;
    defer std.testing.allocator.free(first.json);
    var first_parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, first.json, .{});
    defer first_parsed.deinit();
    try std.testing.expectEqual(Task.max_records_per_event, first_parsed.value.object.get("records").?.array.items.len);
    try std.testing.expect(!first_parsed.value.object.get("terminal").?.bool);
    for (Task.max_records_per_event..35) |index| try task.records.push(.{ .data = try std.fmt.allocPrint(std.testing.allocator, "{{\"index\":{d}}}", .{index}) });
    try task.records.push(.terminal);
    const second = (try task.popRecord()).?;
    defer std.testing.allocator.free(second.json);
    var second_parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, second.json, .{});
    defer second_parsed.deinit();
    try std.testing.expectEqual(@as(usize, 3), second_parsed.value.object.get("records").?.array.items.len);
    try std.testing.expect(second_parsed.value.object.get("terminal").?.bool);
}
