//! JSON encoding for operation data batches and terminal outcomes.
const std = @import("std");
const auth = @import("misa_auth");

pub const Item = struct { json: []u8, terminal: bool = false, terminal_lease: ?u64 = null };

pub const Outcome = struct {
    ok: bool = false,
    status: i64 = 0,
    body: []const u8 = "",
    message: ?[]const u8 = "OperationFailed",
    data: ?std.json.Value = null,
    /// The attempt this call was recorded as, when the transport recorded one.
    attempt_id: ?[]const u8 = null,
    logged_in: bool = false,
    subscription_type: ?[]const u8 = null,
    /// The account a successful authentication result refers to, if any.
    account: ?[]const u8 = null,
    /// Human-readable success message. The completion derives one from the
    /// action when this is absent.
    text: ?[]const u8 = null,
};

pub const Kind = union(enum) {
    auth: struct { action: auth.Action, declaration: auth.Declaration },
    http_stream,
    process_stream,
    file,
    state_load: []const u8,
    state_save,
    conversation_append,
    conversation_load,
    conversation_list,
    conversation_request,
    ordinary,
};

pub fn outcome(a: std.mem.Allocator, completion: []const u8, id: []const u8, kind: Kind, result: Outcome) ![]u8 {
    switch (kind) {
        .auth => |spec| {
            const message = if (!result.ok) result.message orelse "auth failed" else result.text orelse switch (spec.action) {
                .login => "logged in",
                .logout => "logged out",
                .status => if (result.logged_in) "logged in" else "logged out",
                .select => "account selected",
            };
            const account: std.json.Value = if (result.account) |name| .{ .string = name } else .null;
            return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .message = message, .provider = spec.declaration.provider, .logged_in = result.logged_in, .subscription_type = result.subscription_type, .account = account }, .{});
        },
        .http_stream, .process_stream => return terminal(a, completion, id, result.ok, result.status, result.body, result.message, result.attempt_id),
        .file => return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .text = result.body, .message = result.message }, .{}),
        .state_load => |namespace| return std.json.Stringify.valueAlloc(a, .{ .type = completion, .namespace = namespace, .found = result.data != null, .data = result.data orelse .null, .ok = result.ok, .message = result.message }, .{}),
        .state_save => return std.json.Stringify.valueAlloc(a, .{ .type = completion, .ok = result.ok, .message = result.message }, .{}),
        .conversation_append, .conversation_load, .conversation_list, .conversation_request => return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .data = result.data orelse .null, .message = result.message }, .{}),
        .ordinary => {},
    }
    if (result.data) |value| return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .status = result.status, .data = value, .stderr = result.body, .attempt_id = result.attempt_id }, .{});
    return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .status = result.status, .body = result.body, .stdout = result.body, .stderr = result.message orelse "", .message = result.message, .attempt_id = result.attempt_id }, .{});
}

pub fn data(a: std.mem.Allocator, completion: []const u8, id: []const u8, records: []const []u8, terminal_marker: bool) ![]u8 {
    const encoded_type = try std.json.Stringify.valueAlloc(a, completion, .{});
    defer a.free(encoded_type);
    const encoded_id = try std.json.Stringify.valueAlloc(a, id, .{});
    defer a.free(encoded_id);
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(a);
    try out.print(a, "{{\"type\":{s},\"id\":{s},\"phase\":\"data\",\"records\":[", .{ encoded_type, encoded_id });
    for (records, 0..) |record, index| {
        if (index != 0) try out.append(a, ',');
        try out.appendSlice(a, record);
    }
    try out.print(a, "],\"terminal\":{s}}}", .{if (terminal_marker) "true" else "false"});
    return out.toOwnedSlice(a);
}

pub fn terminal(a: std.mem.Allocator, completion: []const u8, id: []const u8, ok: bool, status: i64, body: []const u8, message: ?[]const u8, attempt_id: ?[]const u8) ![]u8 {
    return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .phase = "end", .ok = ok, .status = status, .body = body, .message = message, .attempt_id = attempt_id }, .{});
}

test "data batches encode record JSON without re-encoding it" {
    const records = [_][]u8{@constCast("{\"index\":0}")};
    const json = try data(std.testing.allocator, "stream/done", "quoted-\"id", &records, true);
    defer std.testing.allocator.free(json);
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, json, .{});
    defer parsed.deinit();
    try std.testing.expectEqualStrings("quoted-\"id", parsed.value.object.get("id").?.string);
    try std.testing.expectEqual(@as(i64, 0), parsed.value.object.get("records").?.array.items[0].object.get("index").?.integer);
    try std.testing.expect(parsed.value.object.get("terminal").?.bool);
}

test "auth completions carry their account and message" {
    const declaration: auth.Declaration = .{ .provider = "openai", .strategy = .api_key };
    const selected_json = try outcome(std.testing.allocator, "auth/complete", "auth-1", .{ .auth = .{ .action = .select, .declaration = declaration } }, .{ .ok = true, .message = null, .account = "work", .text = "using account work" });
    defer std.testing.allocator.free(selected_json);
    const selected = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, selected_json, .{});
    defer selected.deinit();
    const object = selected.value.object;
    try std.testing.expectEqualStrings("using account work", object.get("message").?.string);
    try std.testing.expectEqualStrings("work", object.get("account").?.string);

    const listed_json = try outcome(std.testing.allocator, "auth/complete", "auth-2", .{ .auth = .{ .action = .status, .declaration = declaration } }, .{ .ok = true, .message = null, .logged_in = true, .account = "default", .text = "default: logged in\nwork: logged in (active)" });
    defer std.testing.allocator.free(listed_json);
    const listed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, listed_json, .{});
    defer listed.deinit();
    try std.testing.expectEqualStrings("default: logged in\nwork: logged in (active)", listed.value.object.get("message").?.string);

    const plain_json = try outcome(std.testing.allocator, "auth/complete", "auth-3", .{ .auth = .{ .action = .login, .declaration = declaration } }, .{ .ok = true, .message = null, .logged_in = true });
    defer std.testing.allocator.free(plain_json);
    const plain = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, plain_json, .{});
    defer plain.deinit();
    try std.testing.expectEqualStrings("logged in", plain.value.object.get("message").?.string);
    try std.testing.expect(plain.value.object.get("account").? == .null);
}
