//! OAuth device and PKCE flows for subscription providers.
const std = @import("std");

pub const Credential = struct {
    access: []u8,
    refresh: []u8,
    expires: i64,
    account_id: ?[]u8 = null,

    pub fn deinit(self: Credential, allocator: std.mem.Allocator) void {
        allocator.free(self.access);
        allocator.free(self.refresh);
        if (self.account_id) |value| allocator.free(value);
    }
};

const Response = struct {
    status: u16,
    body: []u8,
    fn deinit(self: Response, allocator: std.mem.Allocator) void {
        allocator.free(self.body);
    }
};

const openai_client_id = "app_EMoamEEZ73f0CkXaXp7hrann";
const kimi_client_id = "17e5f671-d194-4dfb-9706-5516cb48c098";

pub fn loginKimi(allocator: std.mem.Allocator, io: std.Io) !Credential {
    const form = try formEncode(allocator, &.{.{ "client_id", kimi_client_id }});
    defer allocator.free(form);
    const started = try post(allocator, io, "https://auth.kimi.com/api/oauth/device_authorization", "application/x-www-form-urlencoded", form);
    defer started.deinit(allocator);
    if (started.status < 200 or started.status >= 300) return error.DeviceAuthorizationFailed;
    var device = try parseObject(allocator, started.body);
    defer device.deinit();
    const object = device.value.object;
    const device_code = string(object, "device_code") orelse return error.InvalidOAuthResponse;
    const user_code = string(object, "user_code") orelse return error.InvalidOAuthResponse;
    const verification = string(object, "verification_uri_complete") orelse string(object, "verification_uri") orelse return error.InvalidOAuthResponse;
    const interval = positiveInteger(object, "interval") orelse 5;
    const expires_in = positiveInteger(object, "expires_in") orelse 900;
    std.debug.print("Open {s}\nCode: {s}\nWaiting for authorization…\n", .{ verification, user_code });

    var elapsed: i64 = 0;
    while (elapsed < expires_in) : (elapsed += interval) {
        io.sleep(.fromSeconds(interval), .awake) catch return error.LoginCancelled;
        const token_form = try formEncode(allocator, &.{
            .{ "client_id", kimi_client_id },
            .{ "device_code", device_code },
            .{ "grant_type", "urn:ietf:params:oauth:grant-type:device_code" },
        });
        defer allocator.free(token_form);
        const response = try post(allocator, io, "https://auth.kimi.com/api/oauth/token", "application/x-www-form-urlencoded", token_form);
        defer response.deinit(allocator);
        var parsed = parseObject(allocator, response.body) catch continue;
        defer parsed.deinit();
        if (response.status >= 200 and response.status < 300 and string(parsed.value.object, "access_token") != null)
            return credentialFromToken(allocator, io, parsed.value.object, null);
        const code = string(parsed.value.object, "error") orelse "";
        if (std.mem.eql(u8, code, "authorization_pending") or std.mem.eql(u8, code, "slow_down")) continue;
        return error.DeviceTokenFailed;
    }
    return error.DeviceAuthorizationExpired;
}

pub fn loginOpenAI(allocator: std.mem.Allocator, io: std.Io) !Credential {
    const request = try std.json.Stringify.valueAlloc(allocator, .{ .client_id = openai_client_id }, .{});
    defer allocator.free(request);
    const started = try post(allocator, io, "https://auth.openai.com/api/accounts/deviceauth/usercode", "application/json", request);
    defer started.deinit(allocator);
    if (started.status < 200 or started.status >= 300) return error.DeviceAuthorizationFailed;
    var device = try parseObject(allocator, started.body);
    defer device.deinit();
    const object = device.value.object;
    const device_id = string(object, "device_auth_id") orelse return error.InvalidOAuthResponse;
    const user_code = string(object, "user_code") orelse return error.InvalidOAuthResponse;
    const interval = positiveInteger(object, "interval") orelse 5;
    std.debug.print("Open https://auth.openai.com/codex/device\nCode: {s}\nWaiting for authorization…\n", .{user_code});

    var elapsed: i64 = 0;
    while (elapsed < 900) : (elapsed += interval) {
        io.sleep(.fromSeconds(interval), .awake) catch return error.LoginCancelled;
        const poll_body = try std.json.Stringify.valueAlloc(allocator, .{ .device_auth_id = device_id, .user_code = user_code }, .{});
        defer allocator.free(poll_body);
        const response = try post(allocator, io, "https://auth.openai.com/api/accounts/deviceauth/token", "application/json", poll_body);
        defer response.deinit(allocator);
        if (response.status == 403 or response.status == 404) continue;
        if (response.status < 200 or response.status >= 300) return error.DeviceTokenFailed;
        var code = try parseObject(allocator, response.body);
        defer code.deinit();
        const authorization_code = string(code.value.object, "authorization_code") orelse return error.InvalidOAuthResponse;
        const verifier = string(code.value.object, "code_verifier") orelse return error.InvalidOAuthResponse;
        const exchange = try formEncode(allocator, &.{
            .{ "grant_type", "authorization_code" },                            .{ "client_id", openai_client_id },
            .{ "code", authorization_code },                                    .{ "code_verifier", verifier },
            .{ "redirect_uri", "https://auth.openai.com/deviceauth/callback" },
        });
        defer allocator.free(exchange);
        const token = try post(allocator, io, "https://auth.openai.com/oauth/token", "application/x-www-form-urlencoded", exchange);
        defer token.deinit(allocator);
        if (token.status < 200 or token.status >= 300) return error.TokenExchangeFailed;
        var parsed = try parseObject(allocator, token.body);
        defer parsed.deinit();
        const access = string(parsed.value.object, "access_token") orelse return error.InvalidOAuthResponse;
        const account = try openAIAccountId(allocator, access);
        errdefer allocator.free(account);
        return credentialFromToken(allocator, io, parsed.value.object, account);
    }
    return error.DeviceAuthorizationExpired;
}

pub fn refresh(allocator: std.mem.Allocator, io: std.Io, provider: []const u8, refresh_token: []const u8) !Credential {
    const endpoint: []const u8 = if (std.mem.eql(u8, provider, "openai-codex"))
        "https://auth.openai.com/oauth/token"
    else if (std.mem.eql(u8, provider, "kimi-coding"))
        "https://auth.kimi.com/api/oauth/token"
    else
        return error.RefreshUnsupported;
    const client_id = if (std.mem.eql(u8, provider, "openai-codex")) openai_client_id else kimi_client_id;
    const body = try formEncode(allocator, &.{
        .{ "grant_type", "refresh_token" }, .{ "client_id", client_id }, .{ "refresh_token", refresh_token },
    });
    defer allocator.free(body);
    const response = try post(allocator, io, endpoint, "application/x-www-form-urlencoded", body);
    defer response.deinit(allocator);
    if (response.status < 200 or response.status >= 300) return error.TokenRefreshFailed;
    var parsed = try parseObject(allocator, response.body);
    defer parsed.deinit();
    const access = string(parsed.value.object, "access_token") orelse return error.InvalidOAuthResponse;
    const account = if (std.mem.eql(u8, provider, "openai-codex")) try openAIAccountId(allocator, access) else null;
    errdefer if (account) |value| allocator.free(value);
    return credentialFromToken(allocator, io, parsed.value.object, account);
}

pub const OpenRouterAuthorization = struct {
    verifier: []u8,
    url: []u8,
    pub fn deinit(self: OpenRouterAuthorization, allocator: std.mem.Allocator) void {
        allocator.free(self.verifier);
        allocator.free(self.url);
    }
};

pub fn startOpenRouter(allocator: std.mem.Allocator, io: std.Io) !OpenRouterAuthorization {
    var random: [32]u8 = undefined;
    try io.randomSecure(&random);
    const encoder = std.base64.url_safe_no_pad.Encoder;
    const verifier = try allocator.alloc(u8, encoder.calcSize(random.len));
    _ = encoder.encode(verifier, &random);
    errdefer allocator.free(verifier);
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(verifier, &digest, .{});
    const challenge = try allocator.alloc(u8, encoder.calcSize(digest.len));
    defer allocator.free(challenge);
    _ = encoder.encode(challenge, &digest);
    const query = try formEncode(allocator, &.{
        .{ "code_challenge", challenge },
        .{ "code_challenge_method", "S256" },
    });
    defer allocator.free(query);
    return .{ .verifier = verifier, .url = try std.fmt.allocPrint(allocator, "https://openrouter.ai/auth?{s}", .{query}) };
}

pub fn finishOpenRouter(allocator: std.mem.Allocator, io: std.Io, verifier: []const u8, input: []const u8) !Credential {
    const code = authorizationCode(input) orelse return error.AuthorizationCodeMissing;
    const body = try std.json.Stringify.valueAlloc(allocator, .{ .code = code, .code_verifier = verifier, .code_challenge_method = "S256" }, .{});
    defer allocator.free(body);
    const response = try post(allocator, io, "https://openrouter.ai/api/v1/auth/keys", "application/json", body);
    defer response.deinit(allocator);
    if (response.status < 200 or response.status >= 300) return error.TokenExchangeFailed;
    var parsed = try parseObject(allocator, response.body);
    defer parsed.deinit();
    const key = string(parsed.value.object, "key") orelse return error.InvalidOAuthResponse;
    const access = try allocator.dupe(u8, key);
    errdefer allocator.free(access);
    return .{
        .access = access,
        .refresh = try allocator.dupe(u8, ""),
        .expires = std.math.maxInt(i64),
    };
}

fn credentialFromToken(allocator: std.mem.Allocator, io: std.Io, object: std.json.ObjectMap, account_id: ?[]u8) !Credential {
    const access = string(object, "access_token") orelse return error.InvalidOAuthResponse;
    const refresh_value = string(object, "refresh_token") orelse return error.InvalidOAuthResponse;
    const expires = positiveInteger(object, "expires_in") orelse return error.InvalidOAuthResponse;
    const access_copy = try allocator.dupe(u8, access);
    errdefer allocator.free(access_copy);
    const refresh_copy = try allocator.dupe(u8, refresh_value);
    return .{
        .access = access_copy,
        .refresh = refresh_copy,
        .expires = std.Io.Clock.real.now(io).toSeconds() + expires,
        .account_id = account_id,
    };
}

fn post(allocator: std.mem.Allocator, io: std.Io, url: []const u8, content_type: []const u8, payload: []const u8) !Response {
    var client: std.http.Client = .{ .allocator = allocator, .io = io };
    defer client.deinit();
    var output: std.Io.Writer.Allocating = .init(allocator);
    errdefer output.deinit();
    const result = try client.fetch(.{
        .location = .{ .url = url },
        .method = .POST,
        .payload = payload,
        .extra_headers = &.{ .{ .name = "content-type", .value = content_type }, .{ .name = "accept", .value = "application/json" } },
        .response_writer = &output.writer,
    });
    return .{ .status = @intFromEnum(result.status), .body = try output.toOwnedSlice() };
}

fn parseObject(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(std.json.Value) {
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source, .{ .allocate = .alloc_always });
    errdefer parsed.deinit();
    if (parsed.value != .object) return error.InvalidOAuthResponse;
    return parsed;
}

fn string(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |item| item,
        else => null,
    };
}

fn positiveInteger(object: std.json.ObjectMap, name: []const u8) ?i64 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .integer => |item| if (item > 0) item else null,
        .float => |item| if (item > 0 and @trunc(item) == item) @intFromFloat(item) else null,
        .string => |item| std.fmt.parseInt(i64, item, 10) catch null,
        else => null,
    };
}

fn formEncode(allocator: std.mem.Allocator, fields: []const struct { []const u8, []const u8 }) ![]u8 {
    var result: std.ArrayList(u8) = .empty;
    errdefer result.deinit(allocator);
    for (fields, 0..) |field, index| {
        if (index != 0) try result.append(allocator, '&');
        try percentEncode(&result, allocator, field[0]);
        try result.append(allocator, '=');
        try percentEncode(&result, allocator, field[1]);
    }
    return result.toOwnedSlice(allocator);
}

fn percentEncode(out: *std.ArrayList(u8), allocator: std.mem.Allocator, value: []const u8) !void {
    const hex = "0123456789ABCDEF";
    for (value) |byte| {
        if (std.ascii.isAlphanumeric(byte) or byte == '-' or byte == '_' or byte == '.' or byte == '~')
            try out.append(allocator, byte)
        else
            try out.appendSlice(allocator, &.{ '%', hex[byte >> 4], hex[byte & 15] });
    }
}

fn authorizationCode(input: []const u8) ?[]const u8 {
    const trimmed = std.mem.trim(u8, input, " \t\r\n");
    if (std.mem.indexOf(u8, trimmed, "code=")) |start| {
        const rest = trimmed[start + 5 ..];
        return rest[0 .. std.mem.indexOfAny(u8, rest, "&#") orelse rest.len];
    }
    return if (trimmed.len == 0) null else trimmed;
}

fn openAIAccountId(allocator: std.mem.Allocator, access: []const u8) ![]u8 {
    var parts = std.mem.splitScalar(u8, access, '.');
    _ = parts.next() orelse return error.InvalidAccessToken;
    const payload = parts.next() orelse return error.InvalidAccessToken;
    const decoder = std.base64.url_safe_no_pad.Decoder;
    const size = try decoder.calcSizeForSlice(payload);
    const decoded = try allocator.alloc(u8, size);
    defer allocator.free(decoded);
    try decoder.decode(decoded, payload);
    var parsed = try parseObject(allocator, decoded);
    defer parsed.deinit();
    const claim = switch (parsed.value.object.get("https://api.openai.com/auth") orelse return error.AccountIdMissing) {
        .object => |object| object,
        else => return error.AccountIdMissing,
    };
    return allocator.dupe(u8, string(claim, "chatgpt_account_id") orelse return error.AccountIdMissing);
}

test "authorization input accepts redirect URLs and bare codes" {
    try std.testing.expectEqualStrings("abc", authorizationCode("http://localhost/callback?code=abc&state=x").?);
    try std.testing.expectEqualStrings("abc", authorizationCode(" abc\n").?);
}
