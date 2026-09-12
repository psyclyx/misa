//! Parse and validate the fixed native effect contract before execution.
const std = @import("std");
const auth = @import("misa_auth");
const file = @import("misa_file");
const image = @import("misa_image");
const syntax = @import("misa_syntax");
const http = @import("misa_http");
const process = @import("misa_process");
const state = @import("misa_state");
const conversation = @import("misa_conversation");
const terminal = @import("misa_terminal");
const timer = @import("timer.zig");
const protected_input = @import("protected_input.zig");

pub const JsonDecode = struct { source: []const u8, completion: []const u8, id: []const u8 };
pub const StateLoad = struct { namespace: []const u8, completion: []const u8 };
pub const StateSave = struct { namespace: []const u8, data: std.json.Value };
pub const ConversationAppend = struct {
    conversation: []const u8,
    entries: []const std.json.Value,
    metadata: ?std.json.Value = null,
    completion: []const u8,
    id: []const u8,
};
pub const ConversationLoad = struct {
    conversation: []const u8,
    after_seq: i64,
    limit: usize,
    completion: []const u8,
    id: []const u8,
};
pub const ConversationList = struct {
    limit: usize,
    completion: []const u8,
    id: []const u8,
};
/// One attempt as policy states it. `request.id` names both the attempt and
/// the running operation, so the completion event returns the id the policy
/// used and no second identity is invented.
pub const ConversationRequest = struct {
    request: conversation.Request,
    completion: []const u8,
};
pub const AuthCommand = struct { action: auth.Action, declaration: auth.Declaration, account: ?[]const u8, completion: []const u8, interaction: []const u8, id: []const u8 };
pub const AuthRespond = struct { id: []const u8, correlation: []const u8, action: []const u8, value: []const u8 };
pub const CancelOperation = struct { id: []const u8 };
pub const FinishOperation = struct { id: []const u8 };

/// What a provider call is recorded as. The effect that asks for the call
/// declares it, because only that layer knows whether the call is a step of the
/// conversation, a policy's side call, or an adapter probing a provider.
pub const Attempt = struct {
    conversation: ?[]const u8 = null,
    parent_request_id: ?[]const u8 = null,
    provider_id: ?[]const u8 = null,
    kind: []const u8 = "turn",
    provider: []const u8,
    model: []const u8,
};

/// A model call over a provider process, with the attempt it is recorded as.
pub const ProviderCall = struct { spec: process.Spec, attempt: Attempt };

/// An HTTP request that may be a model call. A provider's protocol uses this
/// transport too, while a search backend uses it without an attempt, so the
/// attempt is optional here and required by the boundary that names providers.
pub const HttpCall = struct { spec: http.Spec, attempt: ?Attempt = null };

pub const Effect = union(enum) {
    dispatch: std.json.Value,
    terminal_read,
    clipboard_write: []const u8,
    input_protected: protected_input.Spec,
    view_commit: std.json.Value,
    app_quit,
    process_run: process.Spec,
    provider_process: ProviderCall,
    http_request: HttpCall,
    file: file.Spec,
    image: image.Spec,
    syntax_highlight: syntax.Spec,
    json_decode: JsonDecode,
    auth_command: AuthCommand,
    auth_respond: AuthRespond,
    state_load: StateLoad,
    state_save: StateSave,
    conversation_append: ConversationAppend,
    conversation_load: ConversationLoad,
    conversation_list: ConversationList,
    conversation_request: ConversationRequest,
    operation_cancel: CancelOperation,
    operation_finish: FinishOperation,
    timer_start: timer.Start,
    timer_stop: timer.Stop,

    pub fn parse(value: std.json.Value) !Effect {
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
        if (std.mem.eql(u8, kind, "input/protected")) return .{ .input_protected = .{
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            .correlation = nonEmptyStringField(object, "correlation") orelse return error.InvalidEffect,
            .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
        } };
        if (std.mem.eql(u8, kind, "clipboard/write")) {
            const text = stringField(object, "text") orelse return error.InvalidEffect;
            if (text.len > 1024 * 1024) return error.InvalidEffect;
            return .{ .clipboard_write = text };
        }
        if (std.mem.eql(u8, kind, "view/commit")) {
            const lines = object.get("lines") orelse return error.InvalidEffect;
            try terminal.validateLines(lines);
            return .{ .view_commit = lines };
        }
        if (std.mem.eql(u8, kind, "app/quit")) return .app_quit;
        if (std.mem.eql(u8, kind, "operation/cancel")) return .{ .operation_cancel = .{
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
        } };
        if (std.mem.eql(u8, kind, "operation/finish")) return .{ .operation_finish = .{
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
        } };
        if (std.mem.eql(u8, kind, "timer/start")) {
            const interval = object.get("interval_ms") orelse return error.InvalidEffect;
            if (interval != .integer or interval.integer < 10 or interval.integer > 60_000) return error.InvalidEffect;
            return .{ .timer_start = .{
                .interval_ms = @intCast(interval.integer),
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
                .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "timer/stop")) return .{ .timer_stop = .{
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
        } };
        if (std.mem.eql(u8, kind, "process/run")) {
            // A tool process is not a model call: an attempt here would be
            // ignored, and silently ignoring it is worse than refusing it.
            if (object.get("attempt") != null) return error.InvalidEffect;
            return .{ .process_run = try .parse(object) };
        }
        if (std.mem.eql(u8, kind, "provider/process")) {
            // A provider call must say what it is recorded as. This is the
            // boundary that can tell a provider process from a tool process, so
            // it is the boundary that requires the attempt.
            const declared = (try attemptField(object)) orelse return error.InvalidEffect;
            return .{ .provider_process = .{ .spec = try .parse(object), .attempt = declared } };
        }
        if (std.mem.eql(u8, kind, "http/request")) return .{ .http_request = .{ .spec = try .parse(object), .attempt = try attemptField(object) } };
        if (std.mem.eql(u8, kind, "syntax/highlight")) return .{ .syntax_highlight = try .parse(object) };
        if (std.mem.startsWith(u8, kind, "image/")) return .{ .image = try .parse(kind, object) };
        if (std.mem.startsWith(u8, kind, "file/")) return .{ .file = try .parse(kind, object) };
        if (std.mem.eql(u8, kind, "auth/command")) {
            const action_name = nonEmptyStringField(object, "action") orelse return error.InvalidEffect;
            const action: auth.Action = if (std.mem.eql(u8, action_name, "login")) .login else if (std.mem.eql(u8, action_name, "logout")) .logout else if (std.mem.eql(u8, action_name, "status")) .status else if (std.mem.eql(u8, action_name, "select")) .select else return error.InvalidEffect;
            // An account name is optional; when present it must be storable.
            const account: ?[]const u8 = if (object.get("account")) |candidate| blk: {
                if (candidate != .string or !auth.validAccountName(candidate.string)) return error.InvalidEffect;
                break :blk candidate.string;
            } else null;
            return .{ .auth_command = .{
                .action = action,
                .declaration = try auth.Declaration.parse(nonEmptyStringField(object, "provider") orelse return error.InvalidEffect, nonEmptyStringField(object, "strategy") orelse return error.InvalidEffect, object.get("profile") orelse .null),
                .account = account,
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
                .interaction = nonEmptyStringField(object, "interaction") orelse return error.InvalidEffect,
                .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "auth/respond")) return .{ .auth_respond = .{
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            .correlation = nonEmptyStringField(object, "correlation") orelse return error.InvalidEffect,
            .action = nonEmptyStringField(object, "action") orelse return error.InvalidEffect,
            .value = stringField(object, "value") orelse return error.InvalidEffect,
        } };
        if (std.mem.eql(u8, kind, "json/decode")) return .{ .json_decode = .{
            .source = stringField(object, "source") orelse return error.InvalidEffect,
            .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
            .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
        } };
        if (std.mem.eql(u8, kind, "state/load")) {
            const namespace = nonEmptyStringField(object, "namespace") orelse return error.InvalidEffect;
            state.validateNamespace(namespace) catch return error.InvalidEffect;
            return .{ .state_load = .{
                .namespace = namespace,
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "state/save")) {
            const namespace = nonEmptyStringField(object, "namespace") orelse return error.InvalidEffect;
            state.validateNamespace(namespace) catch return error.InvalidEffect;
            return .{ .state_save = .{
                .namespace = namespace,
                .data = object.get("data") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "conversation/append")) {
            const conversation_id = nonEmptyStringField(object, "conversation") orelse return error.InvalidEffect;
            conversation.validateConversationId(conversation_id) catch return error.InvalidEffect;
            const entries = switch (object.get("entries") orelse return error.InvalidEffect) {
                .array => |array| array.items,
                else => return error.InvalidEffect,
            };
            if (entries.len == 0 or entries.len > conversation.max_entries_per_append) return error.InvalidEffect;
            for (entries) |entry| {
                const entry_object = switch (entry) {
                    .object => |item| item,
                    else => return error.InvalidEffect,
                };
                const entry_kind = nonEmptyStringField(entry_object, "kind") orelse return error.InvalidEffect;
                conversation.validateKind(entry_kind) catch return error.InvalidEffect;
                if (entry_object.get("data") == null) return error.InvalidEffect;
            }
            const metadata = object.get("metadata");
            if (metadata != null and metadata.? != .object) return error.InvalidEffect;
            return .{ .conversation_append = .{
                .conversation = conversation_id,
                .entries = entries,
                .metadata = metadata,
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
                .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "conversation/load")) {
            const conversation_id = nonEmptyStringField(object, "conversation") orelse return error.InvalidEffect;
            conversation.validateConversationId(conversation_id) catch return error.InvalidEffect;
            return .{ .conversation_load = .{
                .conversation = conversation_id,
                .after_seq = boundedIntegerField(object, "after_seq", 0, std.math.maxInt(i64), 0) orelse return error.InvalidEffect,
                .limit = @intCast(boundedIntegerField(object, "limit", 1, conversation.max_load_limit, conversation.max_load_limit) orelse return error.InvalidEffect),
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
                .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "conversation/list")) {
            return .{ .conversation_list = .{
                .limit = @intCast(boundedIntegerField(object, "limit", 1, conversation.max_list_limit, conversation.max_list_limit) orelse return error.InvalidEffect),
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
                .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
            } };
        }
        if (std.mem.eql(u8, kind, "conversation/request")) {
            var request: conversation.Request = .{
                .id = nonEmptyStringField(object, "id") orelse return error.InvalidEffect,
                .provider = nonEmptyStringField(object, "provider") orelse return error.InvalidEffect,
                .model = nonEmptyStringField(object, "model") orelse return error.InvalidEffect,
                .status = nonEmptyStringField(object, "status") orelse return error.InvalidEffect,
            };
            if (stringField(object, "conversation")) |conversation_id| {
                conversation.validateConversationId(conversation_id) catch return error.InvalidEffect;
                request.conversation = conversation_id;
            }
            if (stringField(object, "kind")) |attempt_kind| {
                conversation.validateAttemptKind(attempt_kind) catch return error.InvalidEffect;
                request.kind = attempt_kind;
            }
            if (stringField(object, "provider_id")) |field| request.provider_id = field;
            if (stringField(object, "parent_request_id")) |field| request.parent_request_id = field;
            if (stringField(object, "cost_kind")) |field| request.cost_kind = field;
            if (stringField(object, "cost_currency")) |field| request.cost_currency = field;
            request.cost_micros = try optionalCountField(object, "cost_micros");
            request.input_tokens = try optionalCountField(object, "input_tokens");
            request.output_tokens = try optionalCountField(object, "output_tokens");
            request.cache_read_tokens = try optionalCountField(object, "cache_read_tokens");
            request.cache_write_tokens = try optionalCountField(object, "cache_write_tokens");
            request.ttft_ms = try optionalCountField(object, "ttft_ms");
            request.finished_at_ms = try optionalCountField(object, "finished_at_ms");
            if (object.get("settings")) |document| request.settings = document;
            if (object.get("usage")) |document| request.usage = document;
            if (object.get("cost")) |document| request.cost = document;
            if (object.get("metadata")) |document| request.metadata = document;
            conversation.validateRequest(request) catch return error.InvalidEffect;
            return .{ .conversation_request = .{
                .request = request,
                .completion = nonEmptyStringField(object, "completion") orelse return error.InvalidEffect,
            } };
        }
        return error.UnknownNativeEffect;
    }
};

/// Read the attempt a call is recorded as, or null when the effect declares
/// none. Every field the store needs is checked here, so a malformed attempt
/// fails at the boundary rather than inside the worker that writes it.
fn attemptField(object: std.json.ObjectMap) !?Attempt {
    const value = object.get("attempt") orelse return null;
    if (value != .object) return error.InvalidEffect;
    const attempt = value.object;
    var declared: Attempt = .{
        .provider = nonEmptyStringField(attempt, "provider") orelse return error.InvalidEffect,
        .model = nonEmptyStringField(attempt, "model") orelse return error.InvalidEffect,
    };
    conversation.validateLabel(declared.provider) catch return error.InvalidEffect;
    conversation.validateLabel(declared.model) catch return error.InvalidEffect;
    if (stringField(attempt, "conversation")) |id| {
        conversation.validateConversationId(id) catch return error.InvalidEffect;
        declared.conversation = id;
    }
    if (stringField(attempt, "kind")) |name| {
        conversation.validateAttemptKind(name) catch return error.InvalidEffect;
        declared.kind = name;
    }
    if (stringField(attempt, "parent_request_id")) |id| {
        conversation.validateRequestId(id) catch return error.InvalidEffect;
        declared.parent_request_id = id;
    }
    if (stringField(attempt, "provider_id")) |id| {
        conversation.validateLabel(id) catch return error.InvalidEffect;
        declared.provider_id = id;
    }
    return declared;
}

/// Read an optional count: an absent or null field is nothing, and anything
/// that is not a nonnegative integer is an invalid effect.
fn optionalCountField(object: std.json.ObjectMap, name: []const u8) !?i64 {
    const value = object.get(name) orelse return null;
    if (value == .null) return null;
    if (value != .integer or value.integer < 0) return error.InvalidEffect;
    return value.integer;
}

/// Read an optional bounded integer field; null reports an invalid value.
fn boundedIntegerField(object: std.json.ObjectMap, name: []const u8, minimum: i64, maximum: i64, default: i64) ?i64 {
    const value = object.get(name) orelse return default;
    if (value != .integer or value.integer < minimum or value.integer > maximum) return null;
    return value.integer;
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

test "validation covers the whole native contract" {
    var clipboard = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"clipboard/write\",\"text\":\"line one\\nline two\"}", .{});
    defer clipboard.deinit();
    try std.testing.expectEqualStrings("line one\nline two", (try Effect.parse(clipboard.value)).clipboard_write);
    const oversized = try std.testing.allocator.alloc(u8, 1024 * 1024 + 1);
    defer std.testing.allocator.free(oversized);
    try clipboard.value.object.put(std.testing.allocator, "text", .{ .string = oversized });
    try std.testing.expectError(error.InvalidEffect, Effect.parse(clipboard.value));
    var valid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\",\"arg\"],\"completion\":\"done\",\"id\":\"1\"}", .{});
    defer valid.deinit();
    _ = try Effect.parse(valid.value);
    var stream = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\"],\"completion\":\"done\",\"id\":\"1\",\"stdout_format\":\"json_lines_stream\"}", .{});
    defer stream.deinit();
    const stream_effect = try Effect.parse(stream.value);
    try std.testing.expectEqual(process.StdoutFormat.json_lines_stream, stream_effect.process_run.stdout_format);

    // A provider call states the attempt it is recorded as; the attempt is what
    // makes the call accountable, so a provider process without one is refused,
    // and a tool process carrying one is refused rather than ignored.
    var call = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"provider/process\",\"argv\":[\"claude\"],\"completion\":\"provider/claude-complete\",\"id\":\"agent-1\",\"stdout_format\":\"json_lines_stream\",\"attempt\":{\"conversation\":\"chat\",\"kind\":\"turn\",\"provider\":\"claude\",\"model\":\"claude-opus-5\"}}", .{});
    defer call.deinit();
    const provider_call = (try Effect.parse(call.value)).provider_process;
    try std.testing.expectEqualStrings("chat", provider_call.attempt.conversation.?);
    try std.testing.expectEqualStrings("turn", provider_call.attempt.kind);
    try std.testing.expectEqualStrings("claude-opus-5", provider_call.attempt.model);
    var unrecorded = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"provider/process\",\"argv\":[\"claude\"],\"completion\":\"provider/claude-complete\",\"id\":\"agent-1\"}", .{});
    defer unrecorded.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(unrecorded.value));
    var mislabelled = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"provider/process\",\"argv\":[\"claude\"],\"completion\":\"provider/claude-complete\",\"id\":\"agent-1\",\"attempt\":{\"kind\":\"compaction\",\"provider\":\"claude\",\"model\":\"claude-opus-5\"}}", .{});
    defer mislabelled.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(mislabelled.value));
    var tool_attempt = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\"],\"completion\":\"done\",\"id\":\"1\",\"attempt\":{\"provider\":\"claude\",\"model\":\"m\"}}", .{});
    defer tool_attempt.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(tool_attempt.value));
    // An HTTP request may be a model call or a search; only the effect's name
    // says which, so the attempt is optional at this boundary.
    var search = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"http/request\",\"url\":\"https://example.test/search\",\"method\":\"GET\",\"completion\":\"search/done\",\"id\":\"search-1\"}", .{});
    defer search.deinit();
    try std.testing.expect((try Effect.parse(search.value)).http_request.attempt == null);
    var invalid = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"process/run\",\"argv\":[\"tool\"],\"completion\":\"\",\"id\":\"\"}", .{});
    defer invalid.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(invalid.value));
    var cancel = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"operation/cancel\",\"id\":\"request-1\"}", .{});
    defer cancel.deinit();
    _ = try Effect.parse(cancel.value);
    var finish = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"operation/finish\",\"id\":\"request-1\"}", .{});
    defer finish.deinit();
    _ = try Effect.parse(finish.value);
    var timer_effect_json = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"timer/start\",\"interval_ms\":80,\"completion\":\"ui/tick\",\"id\":\"animation\"}", .{});
    defer timer_effect_json.deinit();
    const timer_effect = try Effect.parse(timer_effect_json.value);
    try std.testing.expectEqual(@as(u64, 80), timer_effect.timer_start.interval_ms);
    var bad_timer = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"timer/start\",\"interval_ms\":0,\"completion\":\"ui/tick\",\"id\":\"animation\"}", .{});
    defer bad_timer.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_timer.value));
    var bad_view = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"view/commit\",\"lines\":[{\"spans\":[{\"text\":\"bad\\n\"}]}]}", .{});
    defer bad_view.deinit();
    try std.testing.expectError(error.InvalidView, Effect.parse(bad_view.value));

    var auth_effect = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"auth/command\",\"action\":\"login\",\"provider\":\"kimi-coding\",\"strategy\":\"device_oauth\",\"profile\":{\"id\":\"global\",\"authorization_url\":\"https://auth.kimi.ai/api/oauth/device_authorization\",\"token_url\":\"https://auth.kimi.ai/api/oauth/token\",\"api_base\":\"https://api.kimi.ai/coding/v1\"},\"completion\":\"done\",\"interaction\":\"progress\",\"id\":\"auth-1\"}", .{});
    defer auth_effect.deinit();
    _ = try Effect.parse(auth_effect.value);
    var named_auth = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"auth/command\",\"action\":\"select\",\"provider\":\"openai\",\"strategy\":\"api_key\",\"account\":\"work\",\"completion\":\"done\",\"interaction\":\"progress\",\"id\":\"auth-2\"}", .{});
    defer named_auth.deinit();
    const selected = try Effect.parse(named_auth.value);
    try std.testing.expectEqual(auth.Action.select, selected.auth_command.action);
    try std.testing.expectEqualStrings("work", selected.auth_command.account.?);
    var bad_account = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"auth/command\",\"action\":\"login\",\"provider\":\"openai\",\"strategy\":\"api_key\",\"account\":\"two words\",\"completion\":\"done\",\"interaction\":\"progress\",\"id\":\"auth-3\"}", .{});
    defer bad_account.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_account.value));
    var bad_auth = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"auth/command\",\"action\":\"login\",\"provider\":\"kimi-coding\",\"strategy\":\"device_oauth\",\"profile\":{\"id\":\"global\",\"authorization_url\":\"https://evil.example\",\"token_url\":\"https://auth.kimi.ai/api/oauth/token\",\"api_base\":\"https://api.kimi.ai/coding/v1\"},\"completion\":\"done\",\"interaction\":\"progress\",\"id\":\"auth-1\"}", .{});
    defer bad_auth.deinit();
    try std.testing.expectError(error.UntrustedAuthDeclaration, Effect.parse(bad_auth.value));

    var state_save = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"state/save\",\"namespace\":\"preferences\",\"data\":{}}", .{});
    defer state_save.deinit();
    _ = try Effect.parse(state_save.value);
    var bad_state = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"state/load\",\"namespace\":\"preferences\"}", .{});
    defer bad_state.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_state.value));
    var unsafe_state = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"state/load\",\"namespace\":\"../other\",\"completion\":\"done\"}", .{});
    defer unsafe_state.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(unsafe_state.value));
    var append = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/append\",\"conversation\":\"main\",\"entries\":[{\"kind\":\"message\",\"data\":{\"role\":\"user\"}}],\"completion\":\"conversation/appended\",\"id\":\"log-1\"}", .{});
    defer append.deinit();
    const append_effect = try Effect.parse(append.value);
    try std.testing.expectEqualStrings("main", append_effect.conversation_append.conversation);
    try std.testing.expectEqual(@as(usize, 1), append_effect.conversation_append.entries.len);
    var bad_append = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/append\",\"conversation\":\"has space\",\"entries\":[{\"kind\":\"message\",\"data\":null}],\"completion\":\"conversation/appended\",\"id\":\"log-1\"}", .{});
    defer bad_append.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_append.value));
    var empty_append = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/append\",\"conversation\":\"main\",\"entries\":[],\"completion\":\"conversation/appended\",\"id\":\"log-1\"}", .{});
    defer empty_append.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(empty_append.value));

    var load = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/load\",\"conversation\":\"main\",\"after_seq\":3,\"limit\":10,\"completion\":\"conversation/loaded\",\"id\":\"load-1\"}", .{});
    defer load.deinit();
    const load_effect = try Effect.parse(load.value);
    try std.testing.expectEqual(@as(i64, 3), load_effect.conversation_load.after_seq);
    try std.testing.expectEqual(@as(usize, 10), load_effect.conversation_load.limit);
    var default_load = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/load\",\"conversation\":\"main\",\"completion\":\"conversation/loaded\",\"id\":\"load-1\"}", .{});
    defer default_load.deinit();
    try std.testing.expectEqual(conversation.max_load_limit, (try Effect.parse(default_load.value)).conversation_load.limit);
    var bad_load = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/load\",\"conversation\":\"has space\",\"completion\":\"conversation/loaded\",\"id\":\"load-1\"}", .{});
    defer bad_load.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_load.value));
    var bad_limit = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/load\",\"conversation\":\"main\",\"limit\":0,\"completion\":\"conversation/loaded\",\"id\":\"load-1\"}", .{});
    defer bad_limit.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_limit.value));

    var list = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/list\",\"limit\":5,\"completion\":\"conversations/listed\",\"id\":\"list-1\"}", .{});
    defer list.deinit();
    try std.testing.expectEqual(@as(usize, 5), (try Effect.parse(list.value)).conversation_list.limit);
    var bad_list = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/list\",\"limit\":4096,\"completion\":\"conversations/listed\",\"id\":\"list-1\"}", .{});
    defer bad_list.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_list.value));

    // An attempt states who called, what for, and how it stands; a kind the
    // store does not know, a negative count, or a missing model is refused
    // before anything is written.
    var attempt = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/request\",\"id\":\"agent-2\",\"conversation\":\"chat\",\"kind\":\"side\",\"provider\":\"claude\",\"model\":\"claude-opus-5\",\"status\":\"ok\",\"input_tokens\":10,\"completion\":\"conversation/requested\"}", .{});
    defer attempt.deinit();
    const parsed_attempt = try Effect.parse(attempt.value);
    try std.testing.expectEqualStrings("agent-2", parsed_attempt.conversation_request.request.id);
    try std.testing.expectEqualStrings("side", parsed_attempt.conversation_request.request.kind.?);
    try std.testing.expectEqual(@as(?i64, 10), parsed_attempt.conversation_request.request.input_tokens);
    var bad_attempt = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/request\",\"id\":\"agent-2\",\"kind\":\"compaction\",\"provider\":\"claude\",\"model\":\"claude-opus-5\",\"status\":\"ok\",\"completion\":\"conversation/requested\"}", .{});
    defer bad_attempt.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(bad_attempt.value));
    var negative = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"type\":\"conversation/request\",\"id\":\"agent-2\",\"provider\":\"claude\",\"model\":\"claude-opus-5\",\"status\":\"ok\",\"output_tokens\":-1,\"completion\":\"conversation/requested\"}", .{});
    defer negative.deinit();
    try std.testing.expectError(error.InvalidEffect, Effect.parse(negative.value));
}
