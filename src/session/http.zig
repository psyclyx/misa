//! Generic HTTP mechanism. Provider policy supplies protocol data while this
//! module injects credentials without exposing secrets to Lua.
const std = @import("std");
const auth = @import("misa_auth");

pub const Spec = struct {
    url: []const u8,
    method: std.http.Method,
    body: ?[]const u8,
    json: ?std.json.Value,
    headers: []const std.json.Value,
    credential: ?Credential,
    completion: []const u8,
    id: []const u8,
    response_format: enum { text, json, sse_json },

    pub const Credential = struct {
        id: []const u8,
        header: []const u8,
        prefix: []const u8,
        metadata_field: ?[]const u8,
        metadata_header: ?[]const u8,
    };

    pub fn parse(object: std.json.ObjectMap) !Spec {
        const url = nonEmptyString(object, "url") orelse return error.InvalidEffect;
        if ((!std.mem.startsWith(u8, url, "https://") and !std.mem.startsWith(u8, url, "http://")) or std.mem.indexOfScalar(u8, url, 0) != null)
            return error.InvalidEffect;
        const method_name = if (object.get("method")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else "POST";
        const method: std.http.Method = if (std.mem.eql(u8, method_name, "POST")) .POST else if (std.mem.eql(u8, method_name, "GET")) .GET else return error.InvalidEffect;
        const body = if (object.get("body")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else null;
        const json = object.get("json");
        if (body != null and json != null) return error.InvalidEffect;
        const headers = if (object.get("headers")) |value| switch (value) {
            .array => |array| array.items,
            else => return error.InvalidEffect,
        } else &.{};
        for (headers) |value| {
            const header = switch (value) {
                .object => |item| item,
                else => return error.InvalidEffect,
            };
            try validateHeader(nonEmptyString(header, "name") orelse return error.InvalidEffect);
            try validateHeader(stringField(header, "value") orelse return error.InvalidEffect);
        }
        const credential: ?Credential = if (object.get("credential")) |value| blk: {
            const item = switch (value) {
                .object => |entry| entry,
                else => return error.InvalidEffect,
            };
            const parsed: Credential = .{
                .id = nonEmptyString(item, "id") orelse return error.InvalidEffect,
                .header = nonEmptyString(item, "header") orelse return error.InvalidEffect,
                .prefix = stringField(item, "prefix") orelse "",
                .metadata_field = stringField(item, "metadata_field"),
                .metadata_header = stringField(item, "metadata_header"),
            };
            if ((parsed.metadata_field == null) != (parsed.metadata_header == null)) return error.InvalidEffect;
            try validateHeader(parsed.header);
            try validateHeader(parsed.prefix);
            if (parsed.metadata_header) |header| try validateHeader(header);
            break :blk parsed;
        } else null;
        const response_format = if (object.get("response_format")) |value| switch (value) {
            .string => |string| string,
            else => return error.InvalidEffect,
        } else "text";
        return .{
            .url = url,
            .method = method,
            .body = body,
            .json = json,
            .headers = headers,
            .credential = credential,
            .completion = nonEmptyString(object, "completion") orelse return error.InvalidEffect,
            .id = nonEmptyString(object, "id") orelse return error.InvalidEffect,
            .response_format = if (std.mem.eql(u8, response_format, "text")) .text else if (std.mem.eql(u8, response_format, "json")) .json else if (std.mem.eql(u8, response_format, "sse_json")) .sse_json else return error.InvalidEffect,
        };
    }
};

pub const Result = struct {
    status: u16,
    body: []u8,

    pub fn deinit(self: Result, allocator: std.mem.Allocator) void {
        allocator.free(self.body);
    }
};

pub fn run(allocator: std.mem.Allocator, io: std.Io, store: ?*auth.Store, spec: Spec) !Result {
    var headers: std.ArrayList(std.http.Header) = .empty;
    defer headers.deinit(allocator);
    for (spec.headers) |value| {
        const object = value.object;
        try headers.append(allocator, .{ .name = object.get("name").?.string, .value = object.get("value").?.string });
    }
    var injected: ?[]u8 = null;
    defer if (injected) |value| allocator.free(value);
    if (spec.credential) |credential| {
        const secret = try (store orelse return error.CredentialStoreUnavailable).access(credential.id);
        injected = try std.mem.concat(allocator, u8, &.{ credential.prefix, secret });
        try headers.append(allocator, .{ .name = credential.header, .value = injected.? });
        if (credential.metadata_field) |field| {
            const value = store.?.getField(credential.id, field) orelse return error.CredentialMetadataMissing;
            try headers.append(allocator, .{ .name = credential.metadata_header.?, .value = value });
        }
    }

    const encoded = if (spec.json) |json| try std.json.Stringify.valueAlloc(allocator, json, .{}) else null;
    defer if (encoded) |value| allocator.free(value);

    var client: std.http.Client = .{ .allocator = allocator, .io = io };
    defer client.deinit();
    var response: std.Io.Writer.Allocating = .init(allocator);
    errdefer response.deinit();
    const fetched = try client.fetch(.{
        .location = .{ .url = spec.url },
        .method = spec.method,
        .payload = encoded orelse spec.body,
        .extra_headers = headers.items,
        .response_writer = &response.writer,
    });
    const body = try response.toOwnedSlice();
    if (body.len > 8 * 1024 * 1024) {
        allocator.free(body);
        return error.HttpResponseTooLarge;
    }
    return .{ .status = @intFromEnum(fetched.status), .body = body };
}

fn validateHeader(value: []const u8) !void {
    if (std.mem.indexOfAny(u8, value, "\r\n\x00") != null) return error.InvalidEffect;
}

fn stringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |string| string,
        else => null,
    };
}

fn nonEmptyString(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = stringField(object, name) orelse return null;
    return if (value.len == 0 or std.mem.indexOfScalar(u8, value, 0) != null) null else value;
}

test "HTTP effect validation keeps credentials referential" {
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator,
        \\{"type":"http/request","url":"https://example.test/v1","headers":[{"name":"content-type","value":"application/json"}],"credential":{"id":"openai","header":"authorization","prefix":"Bearer "},"completion":"done","id":"1","body":"{}"}
    , .{});
    defer parsed.deinit();
    const spec = try Spec.parse(parsed.value.object);
    try std.testing.expectEqualStrings("openai", spec.credential.?.id);
    try std.testing.expectEqual(std.http.Method.POST, spec.method);
}
