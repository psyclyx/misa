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
    logged_in: bool = false,
    subscription_type: ?[]const u8 = null,
};

pub const Kind = union(enum) {
    auth: struct { action: auth.Action, declaration: auth.Declaration },
    http_stream,
    process_stream,
    file,
    state_load: []const u8,
    state_save,
    ordinary,
};

pub fn outcome(a: std.mem.Allocator, completion: []const u8, id: []const u8, kind: Kind, result: Outcome) ![]u8 {
    switch (kind) {
        .auth => |spec| {
            const message = if (!result.ok) result.message orelse "auth failed" else switch (spec.action) {
                .login => "logged in",
                .logout => "logged out",
                .status => if (result.logged_in) "logged in" else "logged out",
            };
            return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .message = message, .provider = spec.declaration.provider, .logged_in = result.logged_in, .subscription_type = result.subscription_type }, .{});
        },
        .http_stream, .process_stream => return terminal(a, completion, id, result.ok, result.status, result.body, result.message),
        .file => return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .text = result.body, .message = result.message }, .{}),
        .state_load => |namespace| return std.json.Stringify.valueAlloc(a, .{ .type = completion, .namespace = namespace, .found = result.data != null, .data = result.data orelse .null, .ok = result.ok, .message = result.message }, .{}),
        .state_save => return std.json.Stringify.valueAlloc(a, .{ .type = completion, .ok = result.ok, .message = result.message }, .{}),
        .ordinary => {},
    }
    if (result.data) |value| return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .status = result.status, .data = value, .stderr = result.body }, .{});
    return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .ok = result.ok, .status = result.status, .body = result.body, .stdout = result.body, .stderr = result.message orelse "", .message = result.message }, .{});
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

pub fn terminal(a: std.mem.Allocator, completion: []const u8, id: []const u8, ok: bool, status: i64, body: []const u8, message: ?[]const u8) ![]u8 {
    return std.json.Stringify.valueAlloc(a, .{ .type = completion, .id = id, .phase = "end", .ok = ok, .status = status, .body = body, .message = message }, .{});
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
