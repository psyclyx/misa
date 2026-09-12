//! One native operation's owned inputs, worker lifetime, interaction, and result.
const std = @import("std");
const auth = @import("misa_auth");
const provider_auth = @import("misa_provider_auth");
const provider_process = @import("misa_provider_process");
const file = @import("misa_file");
const image = @import("misa_image");
const syntax = @import("misa_syntax");
const process = @import("misa_process");
const state = @import("misa_state");
const conversation = @import("misa_conversation");
const native_effect = @import("../native_effect.zig");
const http = @import("misa_http");
const buffered_records = @import("../buffered_records.zig");
const channel_module = @import("channel.zig");
const result_json = @import("result_json.zig");

const AuthSpec = struct { action: auth.Action, declaration: auth.Declaration, account: ?[]const u8, completion: []const u8, interaction: []const u8, id: []const u8, environ: *const std.process.Environ.Map, terminal_lease: ?u64, managed_input: bool };
const StateSpec = struct { namespace: []const u8, completion: []const u8, id: []const u8, data: ?std.json.Value, environ: *const std.process.Environ.Map };
const ConversationSpec = struct { spec: conversation.Spec, environ: *const std.process.Environ.Map };
const ConversationLoadSpec = struct { spec: native_effect.ConversationLoad, environ: *const std.process.Environ.Map };
const ConversationListSpec = struct { spec: native_effect.ConversationList, environ: *const std.process.Environ.Map };
const ConversationRequestSpec = struct { request: conversation.Request, completion: []const u8, environ: *const std.process.Environ.Map };
const HttpSpec = struct { spec: http.Spec, attempt: ?native_effect.Attempt, environ: *const std.process.Environ.Map };
const ImageSpec = struct { spec: image.Spec, environ: *const std.process.Environ.Map };
const SyntaxSpec = struct { spec: syntax.Spec, service: *syntax.Service };
const ProcessRequest = struct {
    spec: process.Spec,
    execution: union(enum) { tool, provider: *const std.process.Environ.Map },
    /// Present exactly when the execution is a provider call: a tool process is
    /// not a model call and declares no attempt.
    attempt: ?native_effect.Attempt = null,
};
const Kind = union(enum) { syntax: SyntaxSpec, image: ImageSpec, http: HttpSpec, process: ProcessRequest, file: file.Spec, state_load: StateSpec, state_save: StateSpec, conversation: ConversationSpec, conversation_load: ConversationLoadSpec, conversation_list: ConversationListSpec, conversation_request: ConversationRequestSpec, auth: AuthSpec };

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
    /// The attempt this task recorded, once it has one, so its completion can
    /// name the row the call is accountable to.
    attempt_id: ?[]const u8 = null,
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
            .environment = source.environment,
        };
        task.kind = .{ .process = .{ .spec = spec, .execution = .tool } };
        try task.prepare(spec.completion, spec.id, .{ .first_byte_ms = spec.timeouts.startup_ms, .idle_ms = spec.timeouts.idle_ms, .overall_ms = spec.timeouts.overall_ms });
        task.timeout_kind = .process;
        return task;
    }

    pub fn createProviderProcess(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, call: native_effect.ProviderCall, environ: *const std.process.Environ.Map) !*Task {
        const task = try createProcess(owner_allocator, io, wakeup, call.spec);
        task.kind.process.execution = .{ .provider = environ };
        task.kind.process.attempt = try cloneAttempt(task.arena.allocator(), call.attempt);
        return task;
    }

    pub fn createHttp(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, call: native_effect.HttpCall, environ: *const std.process.Environ.Map) !*Task {
        const source = call.spec;
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
        task.kind = .{ .http = .{ .spec = spec, .attempt = if (call.attempt) |declared| try cloneAttempt(a, declared) else null, .environ = environ } };
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

    pub fn createSyntax(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: syntax.Spec, service: *syntax.Service) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: syntax.Spec = .{
            .id = try a.dupe(u8, source.id),
            .completion = try a.dupe(u8, source.completion),
            .language = try a.dupe(u8, source.language),
            .source = try a.dupe(u8, source.source),
            .timeout_ms = source.timeout_ms,
        };
        task.kind = .{ .syntax = .{ .spec = spec, .service = service } };
        try task.prepare(spec.completion, spec.id, .{});
        task.timeout_kind = .none;
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

    pub fn createConversation(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: native_effect.ConversationAppend, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const entries = try a.alloc(conversation.Entry, source.entries.len);
        for (source.entries, entries) |raw, *copy| {
            const object = raw.object;
            copy.* = .{
                .kind = try a.dupe(u8, object.get("kind").?.string),
                .data = try cloneJson(a, object.get("data").?),
            };
        }
        const spec: conversation.Spec = .{
            .conversation = try a.dupe(u8, source.conversation),
            .entries = entries,
            .metadata = if (source.metadata) |metadata| try cloneJson(a, metadata) else null,
            .completion = try a.dupe(u8, source.completion),
            .id = try a.dupe(u8, source.id),
        };
        task.kind = .{ .conversation = .{ .spec = spec, .environ = environ } };
        try task.prepare(spec.completion, spec.id, .{});
        // SQLite may block on another process's write lock; that wait is not a
        // transport deadline, so completion is bounded by the store, not here.
        task.timeout_kind = .none;
        return task;
    }

    pub fn createConversationLoad(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: native_effect.ConversationLoad, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: ConversationLoadSpec = .{ .spec = .{
            .conversation = try a.dupe(u8, source.conversation),
            .after_seq = source.after_seq,
            .limit = source.limit,
            .completion = try a.dupe(u8, source.completion),
            .id = try a.dupe(u8, source.id),
        }, .environ = environ };
        task.kind = .{ .conversation_load = spec };
        try task.prepare(spec.spec.completion, spec.spec.id, .{});
        // Opening the database may wait on another process's write lock.
        task.timeout_kind = .none;
        return task;
    }

    pub fn createConversationList(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: native_effect.ConversationList, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: ConversationListSpec = .{ .spec = .{
            .limit = source.limit,
            .completion = try a.dupe(u8, source.completion),
            .id = try a.dupe(u8, source.id),
        }, .environ = environ };
        task.kind = .{ .conversation_list = spec };
        try task.prepare(spec.spec.completion, spec.spec.id, .{});
        task.timeout_kind = .none;
        return task;
    }

    pub fn createConversationRequest(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, source: native_effect.ConversationRequest, environ: *const std.process.Environ.Map) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: ConversationRequestSpec = .{ .request = try cloneRequest(a, source.request), .completion = try a.dupe(u8, source.completion), .environ = environ };
        task.kind = .{ .conversation_request = spec };
        try task.prepare(spec.completion, spec.request.id, .{});
        // SQLite may block on another process's write lock; that wait is not a
        // transport deadline, so completion is bounded by the store, not here.
        task.timeout_kind = .none;
        return task;
    }

    pub fn createAuth(owner_allocator: std.mem.Allocator, io: std.Io, wakeup: channel_module.Wakeup, action: auth.Action, declaration: auth.Declaration, account: ?[]const u8, completion: []const u8, interaction: []const u8, id: []const u8, environ: *const std.process.Environ.Map, terminal_lease: ?u64, managed_input: bool) !*Task {
        const task = try create(owner_allocator, io, wakeup);
        errdefer task.destroy();
        const a = task.arena.allocator();
        const spec: AuthSpec = .{ .action = action, .declaration = try cloneDeclaration(a, declaration), .account = if (account) |name| try a.dupe(u8, name) else null, .completion = try a.dupe(u8, completion), .interaction = try a.dupe(u8, interaction), .id = try a.dupe(u8, id), .environ = environ, .terminal_lease = terminal_lease, .managed_input = managed_input };
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
        // Native effects must never fall back to running on the policy owner.
        // Resource exhaustion is an ordinary operation completion failure.
        self.group.concurrent(self.io, worker, .{self}) catch |err| {
            self.result = .{ .message = @errorName(err) };
            self.done.store(true, .release);
            self.records.wakeup.notify();
        };
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
        if (self.timeout_kind == .none) return null;
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

    /// Settle this task's attempt, if it made one, and remember its id for the
    /// completion event.
    fn settleAttempt(self: *Task, live: *?Attempt, status: []const u8) void {
        if (live.*) |*attempt| {
            attempt.settle(status);
            self.attempt_id = attempt.id;
        }
    }

    pub fn completionItem(self: *Task) !result_json.Item {
        var result = self.result;
        result.attempt_id = self.attempt_id;
        const json = try result_json.outcome(self.owner_allocator, self.completion, self.id, self.resultKind(), result);
        self.owner_allocator.free(self.fallback.?);
        self.fallback = null;
        return .{ .json = json, .terminal = true, .terminal_lease = self.terminalLease() };
    }

    pub fn forcedItem(self: *Task, ok: bool, message: ?[]const u8) !result_json.Item {
        const json = try result_json.terminal(self.owner_allocator, self.completion, self.id, ok, 0, "", message, self.attempt_id);
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
        self.fallback = try result_json.terminal(self.owner_allocator, completion, id, false, 0, "", "OperationFailed", null);
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
            .process => |request| if (request.spec.stdout_format == .json_lines_stream) .process_stream else .ordinary,
            .file => .file,
            .image, .syntax => .ordinary,
            .state_load => |spec| .{ .state_load = spec.namespace },
            .state_save => .state_save,
            .conversation => .conversation_append,
            .conversation_load => .conversation_load,
            .conversation_list => .conversation_list,
            .conversation_request => .conversation_request,
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

    /// Encode a reopened conversation for a `conversation/load` completion.
    fn snapshotJson(self: *Task, snapshot: conversation.Snapshot) !std.json.Value {
        const a = self.arena.allocator();
        var entries = std.json.Array.init(a);
        try entries.ensureTotalCapacity(snapshot.entries.len);
        for (snapshot.entries) |record| {
            var object: std.json.ObjectMap = .empty;
            try object.put(a, "seq", .{ .integer = record.seq });
            try object.put(a, "at_ms", .{ .integer = record.at_ms });
            try object.put(a, "kind", .{ .string = record.kind });
            try object.put(a, "request_id", if (record.request_id) |value| .{ .string = value } else .null);
            try object.put(a, "data", try std.json.parseFromSliceLeaky(std.json.Value, a, record.payload, .{ .allocate = .alloc_always }));
            entries.appendAssumeCapacity(.{ .object = object });
        }
        var object: std.json.ObjectMap = .empty;
        try object.put(a, "conversation", .{ .string = snapshot.conversation });
        try object.put(a, "metadata", try std.json.parseFromSliceLeaky(std.json.Value, a, snapshot.metadata, .{ .allocate = .alloc_always }));
        try object.put(a, "forked_from_id", if (snapshot.forked_from_id) |value| .{ .string = value } else .null);
        try object.put(a, "forked_from_seq", if (snapshot.forked_from_seq) |value| .{ .integer = value } else .null);
        try object.put(a, "entries", .{ .array = entries });
        try object.put(a, "more_entries", .{ .bool = snapshot.more_entries });

        // The attempts this branch issued travel with its transcript, so a
        // policy can project cost, usage, and unfinished work from one read.
        var requests = std.json.Array.init(a);
        try requests.ensureTotalCapacity(snapshot.requests.len);
        for (snapshot.requests) |record| {
            var attempt: std.json.ObjectMap = .empty;
            try attempt.put(a, "id", .{ .string = record.id });
            try attempt.put(a, "conversation", if (record.conversation) |value| .{ .string = value } else .null);
            try attempt.put(a, "message_id", if (record.message_id) |value| .{ .string = value } else .null);
            try attempt.put(a, "provider_id", if (record.provider_id) |value| .{ .string = value } else .null);
            try attempt.put(a, "parent_request_id", if (record.parent_request_id) |value| .{ .string = value } else .null);
            try attempt.put(a, "provider", .{ .string = record.provider });
            try attempt.put(a, "model", .{ .string = record.model });
            try attempt.put(a, "status", .{ .string = record.status });
            try attempt.put(a, "kind", .{ .string = record.kind });
            try attempt.put(a, "cost_kind", .{ .string = record.cost_kind });
            try attempt.put(a, "cost_micros", if (record.cost_micros) |value| .{ .integer = value } else .null);
            try attempt.put(a, "cost_currency", .{ .string = record.cost_currency });
            try attempt.put(a, "input_tokens", if (record.input_tokens) |value| .{ .integer = value } else .null);
            try attempt.put(a, "output_tokens", if (record.output_tokens) |value| .{ .integer = value } else .null);
            try attempt.put(a, "cache_read_tokens", if (record.cache_read_tokens) |value| .{ .integer = value } else .null);
            try attempt.put(a, "cache_write_tokens", if (record.cache_write_tokens) |value| .{ .integer = value } else .null);
            try attempt.put(a, "ttft_ms", if (record.ttft_ms) |value| .{ .integer = value } else .null);
            try attempt.put(a, "finished_at_ms", if (record.finished_at_ms) |value| .{ .integer = value } else .null);
            try attempt.put(a, "created_at_ms", .{ .integer = record.created_at_ms });
            try attempt.put(a, "updated_at_ms", .{ .integer = record.updated_at_ms });
            try attempt.put(a, "settings", try self.storedDocument(record.settings));
            try attempt.put(a, "usage", try self.storedDocument(record.usage));
            try attempt.put(a, "cost", try self.storedDocument(record.cost));
            try attempt.put(a, "metadata", try self.storedDocument(record.metadata));
            requests.appendAssumeCapacity(.{ .object = attempt });
        }
        try object.put(a, "requests", .{ .array = requests });
        try object.put(a, "more_requests", .{ .bool = snapshot.more_requests });
        return .{ .object = object };
    }

    /// Decode one stored JSON document for a completion event. A document this
    /// binary cannot parse reads as absent rather than failing the whole read.
    fn storedDocument(self: *Task, text: ?[]const u8) !std.json.Value {
        const present = text orelse return .null;
        return std.json.parseFromSliceLeaky(std.json.Value, self.arena.allocator(), present, .{ .allocate = .alloc_always }) catch .null;
    }

    /// Encode conversation headers for a `conversation/list` completion.
    fn summariesJson(self: *Task, summaries: []conversation.Summary) !std.json.Value {
        const a = self.arena.allocator();
        var conversations = std.json.Array.init(a);
        try conversations.ensureTotalCapacity(summaries.len);
        for (summaries) |summary| {
            var object: std.json.ObjectMap = .empty;
            try object.put(a, "id", .{ .string = summary.id });
            try object.put(a, "created_at", .{ .integer = summary.created_at });
            try object.put(a, "updated_at", .{ .integer = summary.updated_at });
            try object.put(a, "entry_count", .{ .integer = summary.entry_count });
            try object.put(a, "metadata", try std.json.parseFromSliceLeaky(std.json.Value, a, summary.metadata, .{ .allocate = .alloc_always }));
            try object.put(a, "forked_from_id", if (summary.forked_from_id) |value| .{ .string = value } else .null);
            conversations.appendAssumeCapacity(.{ .object = object });
        }
        var object: std.json.ObjectMap = .empty;
        try object.put(a, "conversations", .{ .array = conversations });
        return .{ .object = object };
    }

    fn worker(self: *Task) std.Io.Cancelable!void {
        defer {
            self.done.store(true, .release);
            self.records.wakeup.notify();
        }
        switch (self.kind) {
            .syntax => |request| {
                const a = self.arena.allocator();
                const captures = request.service.run(a, self.io, request.spec) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                var values: std.array_list.Managed(std.json.Value) = .init(a);
                for (captures, 0..) |capture, index| {
                    if (index % 256 == 0) try self.io.checkCancel();
                    var object: std.json.ObjectMap = .empty;
                    object.put(a, "start_byte", .{ .integer = capture.start_byte }) catch return;
                    object.put(a, "end_byte", .{ .integer = capture.end_byte }) catch return;
                    object.put(a, "capture", .{ .string = capture.capture }) catch return;
                    values.append(.{ .object = object }) catch return;
                }
                self.result = .{ .ok = true, .data = .{ .array = values }, .message = null };
            },
            .image => |request| {
                const value = image.run(self.arena.allocator(), self.io, request.spec, request.environ) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .data = value, .message = null };
            },
            .http => |request| {
                var attempt: ?Attempt = null;
                defer if (attempt) |*live| live.deinit();
                if (request.attempt) |declared| attempt = Attempt.begin(self, declared, request.spec.id, request.environ) catch |err| {
                    // A call that cannot be recorded is not made: the row is
                    // what makes the call accountable.
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                var status: []const u8 = "error";
                defer self.settleAttempt(&attempt, status);
                if (request.spec.response_format == .sse_json_stream) {
                    const run_result = http.requestSse(self.arena.allocator(), self.io, request.environ, request.spec, .{ .context = self, .emit = emitHttpRecord, .activity = noteActivityOpaque }) catch |err| {
                        if (err == error.Canceled) {
                            status = "cancelled";
                            return error.Canceled;
                        }
                        self.result = .{ .message = @errorName(err) };
                        return;
                    };
                    self.result = .{ .ok = run_result.status >= 200 and run_result.status < 300 and run_result.failure == null, .status = run_result.status, .body = run_result.error_body, .message = if (run_result.failure) |failure| @errorName(failure) else null };
                    if (self.result.ok) status = "ok";
                } else {
                    const run_result = http.request(self.arena.allocator(), self.io, request.environ, request.spec, .{ .context = self, .note = noteActivityOpaque }) catch |err| {
                        if (err == error.Canceled) {
                            status = "cancelled";
                            return error.Canceled;
                        }
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
                    if (self.result.ok) status = "ok";
                }
            },
            .process => |request| {
                const spec = request.spec;
                // A provider call is recorded as the attempt it declared, and
                // the row is written before the transport starts.
                var attempt: ?Attempt = null;
                defer if (attempt) |*live| live.deinit();
                const provider_environ: ?*const std.process.Environ.Map = switch (request.execution) {
                    .tool => null,
                    .provider => |map| map,
                };
                if (request.attempt) |declared| if (provider_environ) |map| {
                    attempt = Attempt.begin(self, declared, spec.id, map) catch |err| {
                        // A call that cannot be recorded is not made: the row is
                        // what makes the call accountable.
                        self.result = .{ .message = @errorName(err) };
                        return;
                    };
                };
                var status: []const u8 = "error";
                defer self.settleAttempt(&attempt, status);
                if (spec.stdout_format == .json_lines_stream) {
                    const sink: process.StreamSink = .{ .context = self, .emit = emitProcessRecord, .activity = noteActivityOpaque };
                    const run_result = (switch (request.execution) {
                        .tool => process.runJsonLines(self.arena.allocator(), self.io, spec, sink),
                        .provider => |environ| provider_process.runJsonLines(self.arena.allocator(), self.io, environ, spec, sink),
                    }) catch |err| {
                        if (err == error.Canceled) {
                            status = "cancelled";
                            return error.Canceled;
                        }
                        self.result = .{ .status = -1, .message = @errorName(err) };
                        return;
                    };
                    self.result = .{ .ok = run_result.status == 0, .status = run_result.status, .body = run_result.stderr, .message = null };
                    if (self.result.ok) status = "ok";
                } else {
                    const activity: process.ActivitySink = .{ .context = self, .note = noteActivityOpaque };
                    const run_result = (switch (request.execution) {
                        .tool => process.runWithActivity(self.arena.allocator(), self.io, spec, activity),
                        .provider => |environ| provider_process.runWithActivity(self.arena.allocator(), self.io, environ, spec, activity),
                    }) catch |err| {
                        if (err == error.Canceled) {
                            status = "cancelled";
                            return error.Canceled;
                        }
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
                    if (self.result.ok) status = "ok";
                }
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
            .conversation => |request| {
                const entries = request.spec.entries;
                var store = conversation.Store.open(self.arena.allocator(), self.io, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                defer store.deinit();
                const now_ms = std.Io.Timestamp.now(self.io, .real).toMilliseconds();
                const last = store.append(.{ .conversation = request.spec.conversation, .entries = entries, .metadata = request.spec.metadata, .at_ms = now_ms }) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                const a = self.arena.allocator();
                var object: std.json.ObjectMap = .empty;
                object.put(a, "count", .{ .integer = @intCast(entries.len) }) catch {
                    self.result = .{ .message = "OutOfMemory" };
                    return;
                };
                object.put(a, "last_seq", .{ .integer = last }) catch {
                    self.result = .{ .message = "OutOfMemory" };
                    return;
                };
                self.result = .{ .ok = true, .data = .{ .object = object }, .message = null };
            },
            .conversation_load => |request| {
                var store = conversation.Store.open(self.arena.allocator(), self.io, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                defer store.deinit();
                const snapshot = store.load(self.arena.allocator(), request.spec.conversation, request.spec.after_seq, request.spec.limit, conversation.max_requests_per_load) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                const data = self.snapshotJson(snapshot) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .message = null, .data = data };
            },
            .conversation_list => |request| {
                var store = conversation.Store.open(self.arena.allocator(), self.io, request.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                defer store.deinit();
                const summaries = store.list(self.arena.allocator(), request.spec.limit) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                const data = self.summariesJson(summaries) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .message = null, .data = data };
            },
            .conversation_request => |spec| {
                var store = conversation.Store.open(self.arena.allocator(), self.io, spec.environ) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                defer store.deinit();
                const now_ms = std.Io.Timestamp.now(self.io, .real).toMilliseconds();
                store.recordRequest(spec.request, now_ms) catch |err| {
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .message = null };
            },
            .auth => |spec| {
                const command_result = provider_auth.command(self.arena.allocator(), self.io, spec.environ, spec.action, spec.declaration, .{ .context = self, .emitFn = emitInteraction, .inputFn = awaitInteraction, .protected_input = spec.managed_input }, spec.account) catch |err| {
                    if (err == error.Canceled) return error.Canceled;
                    self.result = .{ .message = @errorName(err) };
                    return;
                };
                self.result = .{ .ok = true, .logged_in = command_result.logged_in, .subscription_type = command_result.subscription_type, .account = command_result.account, .text = command_result.message };
            },
        }
    }
};

fn cloneDeclaration(a: std.mem.Allocator, source: auth.Declaration) !auth.Declaration {
    return .{ .provider = try a.dupe(u8, source.provider), .strategy = source.strategy, .profile_id = if (source.profile_id) |value| try a.dupe(u8, value) else null, .authorization_url = if (source.authorization_url) |value| try a.dupe(u8, value) else null, .token_url = if (source.token_url) |value| try a.dupe(u8, value) else null, .api_base = if (source.api_base) |value| try a.dupe(u8, value) else null, .provision_url = if (source.provision_url) |value| try a.dupe(u8, value) else null };
}

fn cloneJson(a: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    const encoded = try std.json.Stringify.valueAlloc(a, value, .{});
    const parsed = try std.json.parseFromSlice(std.json.Value, a, encoded, .{ .allocate = .alloc_always });
    return parsed.value;
}

/// Copy an attempt declaration into a task's arena: the effect's text belongs
/// to the caller's parsed frame, which is gone by the time the worker runs.
fn cloneAttempt(a: std.mem.Allocator, source: native_effect.Attempt) !native_effect.Attempt {
    return .{
        .conversation = if (source.conversation) |value| try a.dupe(u8, value) else null,
        .parent_request_id = if (source.parent_request_id) |value| try a.dupe(u8, value) else null,
        .provider_id = if (source.provider_id) |value| try a.dupe(u8, value) else null,
        .kind = try a.dupe(u8, source.kind),
        .provider = try a.dupe(u8, source.provider),
        .model = try a.dupe(u8, source.model),
    };
}

/// One recorded attempt, held open for as long as the call runs, so the
/// outcome lands in the row its start wrote.
const Attempt = struct {
    store: conversation.Store,
    declaration: native_effect.Attempt,
    id: []u8,

    /// Write the row before the transport starts. The order is the point: a
    /// call the log cannot account for is exactly what an unfinished attempt
    /// has to be able to report.
    fn begin(task: *Task, declaration: native_effect.Attempt, request_id: []const u8, environ: *const std.process.Environ.Map) !Attempt {
        var store = try conversation.Store.open(task.arena.allocator(), task.io, environ);
        errdefer store.deinit();
        const now_ms = std.Io.Timestamp.now(task.io, .real).toMilliseconds();
        const id = try store.startRequest(.{
            .id = request_id,
            .kind = declaration.kind,
            .conversation = declaration.conversation,
            .parent_request_id = declaration.parent_request_id,
            .provider_id = declaration.provider_id,
            .provider = declaration.provider,
            .model = declaration.model,
            .status = "started",
        }, now_ms);
        return .{ .store = store, .declaration = declaration, .id = id };
    }

    /// Settle the attempt with what the transport knows. Cost is left `unknown`
    /// rather than guessed: policy reports a figure when it has one, and a row
    /// claiming a number nobody reported would be worse than one that says it
    /// does not know. A settle that does not land — a cancelled call can have
    /// its transport taken away mid-flight — leaves the row `started`, which is
    /// what a crash leaves too.
    fn settle(self: *Attempt, status: []const u8) void {
        const now_ms = std.Io.Timestamp.now(self.store.io, .real).toMilliseconds();
        self.store.recordRequest(.{
            .id = self.id,
            .kind = self.declaration.kind,
            .provider = self.declaration.provider,
            .model = self.declaration.model,
            .status = status,
            .cost_kind = "unknown",
            .finished_at_ms = now_ms,
        }, now_ms) catch {};
    }

    fn deinit(self: *Attempt) void {
        self.store.deinit();
    }
};

/// Copy an attempt into a task's arena: the effect's text and documents belong
/// to the caller's parsed frame, which is gone by the time the worker runs.
fn cloneRequest(a: std.mem.Allocator, source: conversation.Request) !conversation.Request {
    return .{
        .id = try a.dupe(u8, source.id),
        .kind = if (source.kind) |value| try a.dupe(u8, value) else null,
        .conversation = if (source.conversation) |value| try a.dupe(u8, value) else null,
        .message_id = if (source.message_id) |value| try a.dupe(u8, value) else null,
        .provider_id = if (source.provider_id) |value| try a.dupe(u8, value) else null,
        .parent_request_id = if (source.parent_request_id) |value| try a.dupe(u8, value) else null,
        .provider = try a.dupe(u8, source.provider),
        .model = try a.dupe(u8, source.model),
        .status = try a.dupe(u8, source.status),
        .cost_kind = try a.dupe(u8, source.cost_kind),
        .cost_micros = source.cost_micros,
        .cost_currency = try a.dupe(u8, source.cost_currency),
        .input_tokens = source.input_tokens,
        .output_tokens = source.output_tokens,
        .cache_read_tokens = source.cache_read_tokens,
        .cache_write_tokens = source.cache_write_tokens,
        .ttft_ms = source.ttft_ms,
        .finished_at_ms = source.finished_at_ms,
        .settings = if (source.settings) |value| try cloneJson(a, value) else null,
        .usage = if (source.usage) |value| try cloneJson(a, value) else null,
        .cost = if (source.cost) |value| try cloneJson(a, value) else null,
        .metadata = if (source.metadata) |value| try cloneJson(a, value) else null,
    };
}

fn cloneFile(a: std.mem.Allocator, source: file.Spec) !file.Spec {
    return switch (source) {
        .read => |value| .{ .read = .{ .path = try a.dupe(u8, value.path), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id), .anchored = value.anchored, .start_line = value.start_line, .max_lines = value.max_lines } },
        .list => |value| .{ .list = .{ .path = try a.dupe(u8, value.path), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
        .write => |value| .{ .write = .{ .path = try a.dupe(u8, value.path), .content = try a.dupe(u8, value.content), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
        .edit => |value| .{ .edit = .{ .path = try a.dupe(u8, value.path), .old = try a.dupe(u8, value.old), .new = try a.dupe(u8, value.new), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
        .edit_lines => |value| .{ .edit_lines = .{ .path = try a.dupe(u8, value.path), .snapshot = try a.dupe(u8, value.snapshot), .start = try a.dupe(u8, value.start), .end = try a.dupe(u8, value.end), .position = value.position, .content = try a.dupe(u8, value.content), .completion = try a.dupe(u8, value.completion), .id = try a.dupe(u8, value.id) } },
    };
}

test "task publishes correlated auth interaction events" {
    const wakeup = try channel_module.Wakeup.init();
    defer wakeup.deinit();
    const task = try Task.createAuth(std.testing.allocator, std.testing.io, wakeup, .login, .{ .provider = "openai-codex", .strategy = .device_oauth, .profile_id = "default", .authorization_url = "https://auth.openai.com/api/accounts/deviceauth/usercode", .token_url = "https://auth.openai.com/oauth/token" }, null, "auth/complete", "auth/interaction", "login:openai", undefined, null, true);
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

test "syntax task owns one source copy before workers run" {
    const allocator = std.testing.allocator;
    const wakeup = try channel_module.Wakeup.init();
    defer wakeup.deinit();
    var service = try syntax.Service.init(allocator, "");
    defer service.deinit();
    var source = [_]u8{ 'a', '=', '1' };
    const task = try Task.createSyntax(allocator, std.testing.io, wakeup, .{ .id = "highlight", .completion = "syntax/done", .language = "lua", .source = &source }, &service);
    defer task.destroy();
    source[0] = 'b';
    try std.testing.expectEqualStrings("a=1", task.kind.syntax.spec.source);
    try std.testing.expect(service.highlighter == null);
}

test "unavailable concurrency completes with failure without running native work inline" {
    const allocator = std.testing.allocator;
    var threaded = std.Io.Threaded.init(allocator, .{ .async_limit = .nothing, .concurrent_limit = .nothing });
    defer threaded.deinit();
    const io = threaded.io();
    const wakeup = try channel_module.Wakeup.init();
    defer wakeup.deinit();
    var service = try syntax.Service.init(allocator, "");
    defer service.deinit();
    const task = try Task.createSyntax(allocator, io, wakeup, .{ .id = "no-worker", .completion = "syntax/done", .language = "unknown", .source = "private source" }, &service);
    defer task.destroy();
    task.start();
    try task.await();
    try std.testing.expect(task.isDone());
    // The worker initializes this cache even for a missing grammar. Keeping it
    // absent proves group submission did not execute the native operation.
    try std.testing.expect(service.highlighter == null);
    var ready = [_]std.posix.pollfd{.{ .fd = wakeup.read_fd, .events = std.posix.POLL.IN, .revents = 0 }};
    try std.testing.expectEqual(@as(usize, 1), try std.posix.poll(&ready, 0));
    const item = try task.completionItem();
    defer allocator.free(item.json);
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, item.json, .{});
    defer parsed.deinit();
    try std.testing.expect(item.terminal);
    try std.testing.expect(!parsed.value.object.get("ok").?.bool);
    try std.testing.expectEqualStrings("no-worker", parsed.value.object.get("id").?.string);
    try std.testing.expectEqualStrings("syntax/done", parsed.value.object.get("type").?.string);
    try std.testing.expectEqualStrings("ConcurrencyUnavailable", parsed.value.object.get("message").?.string);
}
